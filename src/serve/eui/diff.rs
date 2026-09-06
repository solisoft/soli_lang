//! Tree diff: the previous render against the new one, as patch ops.
//!
//! Matched nodes keep their ids, so the client's arena is edited in place.
//! Keyed children are matched by key and reordered with `MoveChild`;
//! positional children are matched by index. A node whose kind changed is
//! replaced whole.

use eui_proto::{Op, Value as WireValue};

use super::tree::{assign_fresh_ids, flatten, Encoder, TNode};

/// Diff `old` against `new`, assigning ids into `new`, appending ops.
pub fn diff(enc: &mut Encoder, old: &TNode, new: &mut TNode, ops: &mut Vec<Op>) {
    if old.kind != new.kind {
        assign_fresh_ids(enc, new);
        ops.push(Op::Replace { node: old.id, subtree: flatten(new) });
        return;
    }
    new.id = old.id;
    let id = new.id;

    if old.style != new.style {
        ops.push(Op::SetStyle { node: id, style: new.style });
    }
    if old.text != new.text {
        if let Some(t) = &new.text {
            ops.push(Op::SetText { node: id, text: t.clone() });
        } else {
            ops.push(Op::SetText { node: id, text: eui_proto::TextRef::Inline(String::new()) });
        }
    }
    for (name, value) in &new.props {
        if old.props.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value) {
            ops.push(Op::SetProp { node: id, prop: *name, value: value.clone() });
        }
    }
    for (name, _) in &old.props {
        if !new.props.iter().any(|(n, _)| n == name) {
            ops.push(Op::SetProp { node: id, prop: *name, value: WireValue::Null });
        }
    }
    for (event, handler) in &new.handlers {
        if old.handlers.iter().find(|(e, _)| e == event).map(|(_, h)| h) != Some(handler) {
            ops.push(Op::SetHandler { node: id, event: *event, handler: *handler });
        }
    }
    for (event, _) in &old.handlers {
        if !new.handlers.iter().any(|(e, _)| e == event) {
            ops.push(Op::ClearHandler { node: id, event: *event });
        }
    }

    let keyed = new.children.iter().any(|c| c.key.is_some()) || old.children.iter().any(|c| c.key.is_some());
    if keyed {
        diff_keyed(enc, id, &old.children, &mut new.children, ops);
    } else {
        diff_positional(enc, id, &old.children, &mut new.children, ops);
    }
}

fn diff_positional(enc: &mut Encoder, parent: u32, old: &[TNode], new: &mut [TNode], ops: &mut Vec<Op>) {
    let common = old.len().min(new.len());
    for i in 0..common {
        diff(enc, &old[i], &mut new[i], ops);
    }
    if old.len() > new.len() {
        ops.push(Op::RemoveChild { parent, index: new.len() as u32, count: (old.len() - new.len()) as u32 });
    }
    for (i, child) in new.iter_mut().enumerate().skip(common) {
        assign_fresh_ids(enc, child);
        ops.push(Op::InsertChild { parent, index: i as u32, subtree: flatten(child) });
    }
}

fn diff_keyed(enc: &mut Encoder, parent: u32, old: &[TNode], new: &mut [TNode], ops: &mut Vec<Op>) {
    // Current child list on the client, as ids, kept in step with each op.
    let mut live: Vec<u32> = old.iter().map(|c| c.id).collect();

    // 1. Remove old children whose key is gone (or unkeyed ones: positional
    //    matching inside a keyed list is not attempted).
    let keep = |o: &TNode| o.key.as_ref().is_some_and(|k| new.iter().any(|n| n.key.as_ref() == Some(k)));
    let mut i = old.len();
    while i > 0 {
        i -= 1;
        if !keep(&old[i]) {
            ops.push(Op::RemoveChild { parent, index: i as u32, count: 1 });
            live.remove(i);
        }
    }

    // 2. Walk the new order: matched keys move into place, new keys insert.
    for (target, child) in new.iter_mut().enumerate() {
        let matched = child.key.as_ref().and_then(|k| old.iter().find(|o| o.key.as_ref() == Some(k)));
        match matched {
            Some(o) => {
                let from = live.iter().position(|id| *id == o.id).unwrap_or(target);
                if from != target {
                    ops.push(Op::MoveChild { parent, from: from as u32, to: target as u32 });
                    let id = live.remove(from);
                    live.insert(target, id);
                }
                diff(enc, o, child, ops);
            }
            None => {
                assign_fresh_ids(enc, child);
                ops.push(Op::InsertChild { parent, index: target as u32, subtree: flatten(child) });
                live.insert(target, child.id);
            }
        }
    }
}
