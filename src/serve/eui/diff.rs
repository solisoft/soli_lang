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

/// Keyed and unkeyed children can share a parent — one keyed value node
/// among plain siblings is the common case. Unkeyed children match by their
/// ordinal among the unkeyed ones, so they are never rebuilt just because a
/// sibling has a key.
fn match_key(node: &TNode, unkeyed_ordinal: &mut u32) -> String {
    match &node.key {
        Some(k) => format!("k:{k}"),
        None => {
            let o = *unkeyed_ordinal;
            *unkeyed_ordinal += 1;
            format!("#{o}")
        }
    }
}

fn diff_keyed(enc: &mut Encoder, parent: u32, old: &[TNode], new: &mut [TNode], ops: &mut Vec<Op>) {
    let mut ord = 0u32;
    let old_keys: Vec<String> = old.iter().map(|n| match_key(n, &mut ord)).collect();
    let mut ord = 0u32;
    let new_keys: Vec<String> = new.iter().map(|n| match_key(n, &mut ord)).collect();

    // Current child list on the client, as ids, kept in step with each op.
    let mut live: Vec<u32> = old.iter().map(|c| c.id).collect();

    // 1. Remove old children whose match key is gone, or whose kind changed
    //    (a kind change is a replace, done as remove + insert).
    let mut i = old.len();
    while i > 0 {
        i -= 1;
        let keep = new_keys.iter().position(|k| *k == old_keys[i]).is_some_and(|j| new[j].kind == old[i].kind);
        if !keep {
            ops.push(Op::RemoveChild { parent, index: i as u32, count: 1 });
            live.remove(i);
        }
    }

    // 2. Walk the new order: matched keys move into place, new keys insert.
    for (target, child) in new.iter_mut().enumerate() {
        let matched = old_keys.iter().position(|k| *k == new_keys[target]).map(|i| &old[i]).filter(|o| o.kind == child.kind);
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

#[cfg(test)]
mod tests {
    use super::*;
    use eui_proto::{NodeKind, TextRef};

    fn leaf(kind: NodeKind, key: Option<&str>, text: &str) -> TNode {
        TNode { id: 0, kind, style: 0, key: key.map(str::to_owned), key_atom: 0, text: Some(TextRef::Inline(text.to_owned())), props: vec![], handlers: vec![], children: vec![] }
    }
    fn parent(children: Vec<TNode>) -> TNode {
        TNode { id: 0, kind: NodeKind::Box, style: 0, key: None, key_atom: 0, text: None, props: vec![], handlers: vec![], children }
    }

    #[test]
    fn one_keyed_child_among_unkeyed_siblings_diffs_in_place() {
        let mut enc = Encoder::default();
        let mut old = parent(vec![leaf(NodeKind::Text, None, "title"), leaf(NodeKind::Text, Some("value"), "0"), leaf(NodeKind::Box, None, "row"), leaf(NodeKind::Text, None, "hint")]);
        assign_fresh_ids(&mut enc, &mut old);
        let mut new = parent(vec![leaf(NodeKind::Text, None, "title"), leaf(NodeKind::Text, Some("value"), "1"), leaf(NodeKind::Box, None, "row"), leaf(NodeKind::Text, None, "hint")]);
        let mut ops = Vec::new();
        diff(&mut enc, &old, &mut new, &mut ops);
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(matches!(&ops[0], Op::SetText { node, .. } if *node == old.children[1].id));
        // Every child kept its id.
        for (o, n) in old.children.iter().zip(new.children.iter()) {
            assert_eq!(o.id, n.id);
        }
    }

    #[test]
    fn a_reversed_keyed_list_is_moves() {
        let mut enc = Encoder::default();
        let mut old = parent((0..5).map(|i| leaf(NodeKind::Box, Some(&i.to_string()), "")).collect());
        assign_fresh_ids(&mut enc, &mut old);
        let mut new = parent((0..5).rev().map(|i| leaf(NodeKind::Box, Some(&i.to_string()), "")).collect());
        let mut ops = Vec::new();
        diff(&mut enc, &old, &mut new, &mut ops);
        assert!(ops.iter().all(|o| matches!(o, Op::MoveChild { .. })), "{ops:?}");
        assert_eq!(ops.len(), 4);
        assert_eq!(new.children[0].id, old.children[4].id);
    }
}
