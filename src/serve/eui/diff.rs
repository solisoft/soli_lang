//! Tree diff: the previous render against the new one, as patch ops.
//!
//! Matched nodes keep their ids, so the client's arena is edited in place.
//! Keyed children are matched by key and reordered with `MoveChild`;
//! positional children are matched by index. A node whose kind changed is
//! replaced whole.

use eui_proto::{Op, Value as WireValue};
use std::collections::HashMap;

use super::tree::{assign_fresh_ids, flatten, Child, Encoder, TNode};

/// Diff `old` against `new`, assigning ids into `new`, appending ops.
pub fn diff(enc: &mut Encoder, old: &TNode, new: &mut TNode, ops: &mut Vec<Op>) {
    if old.kind != new.kind {
        assign_fresh_ids(enc, new);
        ops.push(Op::Replace {
            node: old.id,
            subtree: flatten(new),
        });
        return;
    }
    new.id = old.id;
    let id = new.id;

    if old.style != new.style {
        ops.push(Op::SetStyle {
            node: id,
            style: new.style,
        });
    }
    if old.text != new.text {
        if let Some(t) = &new.text {
            ops.push(Op::SetText {
                node: id,
                text: t.clone(),
            });
        } else {
            ops.push(Op::SetText {
                node: id,
                text: eui_proto::TextRef::Inline(String::new()),
            });
        }
    }
    for (name, value) in &new.props {
        if old.props.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value) {
            ops.push(Op::SetProp {
                node: id,
                prop: *name,
                value: value.clone(),
            });
        }
    }
    for (name, _) in &old.props {
        if !new.props.iter().any(|(n, _)| n == name) {
            ops.push(Op::SetProp {
                node: id,
                prop: *name,
                value: WireValue::Null,
            });
        }
    }
    for (event, handler) in &new.handlers {
        if old
            .handlers
            .iter()
            .find(|(e, _)| e == event)
            .map(|(_, h)| h)
            != Some(handler)
        {
            ops.push(Op::SetHandler {
                node: id,
                event: *event,
                handler: *handler,
            });
        }
    }
    for (event, _) in &old.handlers {
        if !new.handlers.iter().any(|(e, _)| e == event) {
            ops.push(Op::ClearHandler {
                node: id,
                event: *event,
            });
        }
    }

    let keyed = new.children.iter().any(|c| c.key.is_some())
        || old.children.iter().any(|c| c.key.is_some());
    if keyed {
        diff_keyed(enc, id, &old.children, &mut new.children, ops);
    } else {
        diff_positional(enc, id, &old.children, &mut new.children, ops);
    }
}

fn diff_positional(
    enc: &mut Encoder,
    parent: u32,
    old: &[Child],
    new: &mut [Child],
    ops: &mut Vec<Op>,
) {
    let common = old.len().min(new.len());
    for i in 0..common {
        if old[i].same_kept(&new[i]) {
            continue;
        }
        diff(enc, old[i].node(), new[i].node_mut(), ops);
    }
    if old.len() > new.len() {
        ops.push(Op::RemoveChild {
            parent,
            index: new.len() as u32,
            count: (old.len() - new.len()) as u32,
        });
    }
    for (i, child) in new.iter_mut().enumerate().skip(common) {
        let node = child.node_mut();
        assign_fresh_ids(enc, node);
        ops.push(Op::InsertChild {
            parent,
            index: i as u32,
            subtree: flatten(node),
        });
    }
}

/// How a child is matched between renders: by its key, or — unkeyed among
/// keyed siblings — by its ordinal among the unkeyed ones, so it is never
/// rebuilt just because a sibling has a key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MatchKey<'a> {
    Keyed(&'a str),
    Ordinal(u32),
}

fn match_key<'a>(node: &'a TNode, unkeyed_ordinal: &mut u32) -> MatchKey<'a> {
    match &node.key {
        Some(k) => MatchKey::Keyed(k),
        None => {
            let o = *unkeyed_ordinal;
            *unkeyed_ordinal += 1;
            MatchKey::Ordinal(o)
        }
    }
}

/// A Fenwick tree over the kept old children: how many of them, not yet
/// placed, sit before a given old position. That is a child's current index
/// on the client, less the prefix already placed — found in O(log C), where
/// a search of the live list and a shift of everything behind it made a
/// reversed list of C children cost C² element moves.
struct Unplaced {
    tree: Vec<u32>,
}

impl Unplaced {
    fn new(n: usize) -> Self {
        let mut this = Self {
            tree: vec![0; n + 1],
        };
        for i in 0..n {
            this.add(i, 1);
        }
        this
    }

    fn add(&mut self, index: usize, delta: i32) {
        let mut i = index + 1;
        while i < self.tree.len() {
            self.tree[i] = self.tree[i].wrapping_add(delta as u32);
            i += i.isolate_lowest_one();
        }
    }

    /// Unplaced children strictly before `index`.
    fn before(&self, index: usize) -> usize {
        let mut i = index;
        let mut sum = 0u32;
        while i > 0 {
            sum += self.tree[i];
            i -= i.isolate_lowest_one();
        }
        sum as usize
    }
}

fn diff_keyed(enc: &mut Encoder, parent: u32, old: &[Child], new: &mut [Child], ops: &mut Vec<Op>) {
    let mut ord = 0u32;
    let old_keys: Vec<MatchKey> = old.iter().map(|n| match_key(n, &mut ord)).collect();
    let mut ord = 0u32;
    let new_keys: Vec<MatchKey> = new.iter().map(|n| match_key(n, &mut ord)).collect();
    // Key -> first position on each side; a duplicate key matches its first
    // occurrence, as a linear search would.
    let mut old_index: HashMap<MatchKey, usize> = HashMap::with_capacity(old.len());
    for (i, k) in old_keys.iter().enumerate() {
        old_index.entry(*k).or_insert(i);
    }
    let mut new_index: HashMap<MatchKey, usize> = HashMap::with_capacity(new.len());
    for (j, k) in new_keys.iter().enumerate() {
        new_index.entry(*k).or_insert(j);
    }

    // 1. Remove old children whose match key is gone, or whose kind changed
    //    (a kind change is a replace, done as remove + insert). Back to
    //    front, so each index is still the client's at that point, and a
    //    run of removals is one op — the op carries a count for that.
    let mut kept_old: Vec<usize> = Vec::with_capacity(old.len());
    let mut run: Option<(usize, u32)> = None; // (first index, count)
    let mut i = old.len();
    while i > 0 {
        i -= 1;
        let keep = new_index
            .get(&old_keys[i])
            .is_some_and(|&j| new[j].kind == old[i].kind);
        if keep {
            kept_old.push(i);
            if let Some((first, count)) = run.take() {
                ops.push(Op::RemoveChild {
                    parent,
                    index: first as u32,
                    count,
                });
            }
        } else {
            run = Some(match run {
                Some((_, count)) => (i, count + 1),
                None => (i, 1),
            });
        }
    }
    if let Some((first, count)) = run {
        ops.push(Op::RemoveChild {
            parent,
            index: first as u32,
            count,
        });
    }
    kept_old.reverse();
    // Old index -> rank among the kept, which is the child's position on the
    // client after the removals.
    let mut rank_of: HashMap<usize, usize> = HashMap::with_capacity(kept_old.len());
    for (rank, old_i) in kept_old.iter().enumerate() {
        rank_of.insert(*old_i, rank);
    }
    let mut unplaced = Unplaced::new(kept_old.len());

    // 2. Walk the new order. After `placed` children are settled, the client
    //    list is those, then the kept old children not yet placed, in old
    //    order — so a matched child's index is `placed` plus the unplaced
    //    kept children before it, and it is in place already when no
    //    unplaced child precedes it.
    // Which old child each new one matches, settled before the new list is
    // borrowed for mutation.
    let matched_old: Vec<Option<usize>> = new_keys
        .iter()
        .enumerate()
        .map(|(j, k)| {
            old_index
                .get(k)
                .copied()
                .filter(|&i| old[i].kind == new[j].kind)
        })
        .collect();
    drop(new_keys);
    let mut placed = 0usize;
    for (target, child) in new.iter_mut().enumerate() {
        match matched_old[target].map(|i| (i, &old[i])) {
            Some((old_i, o)) => {
                let rank = rank_of[&old_i];
                let ahead = unplaced.before(rank);
                let from = placed + ahead;
                if ahead != 0 {
                    ops.push(Op::MoveChild {
                        parent,
                        from: from as u32,
                        to: placed as u32,
                    });
                }
                unplaced.add(rank, -1);
                placed += 1;
                // The same kept node on both sides: nothing to compare.
                if !o.same_kept(child) {
                    diff(enc, o.node(), child.node_mut(), ops);
                }
            }
            None => {
                let node = child.node_mut();
                assign_fresh_ids(enc, node);
                ops.push(Op::InsertChild {
                    parent,
                    index: placed as u32,
                    subtree: flatten(node),
                });
                placed += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eui_proto::{NodeKind, TextRef};

    fn leaf(kind: NodeKind, key: Option<&str>, text: &str) -> TNode {
        TNode {
            id: 0,
            kind,
            style: 0,
            key: key.map(str::to_owned),
            key_atom: 0,
            text: Some(TextRef::Inline(text.to_owned())),
            props: vec![],
            handlers: vec![],
            children: vec![],
            identity: 0,
            size: 0,
        }
    }
    fn parent(children: Vec<TNode>) -> TNode {
        TNode {
            id: 0,
            kind: NodeKind::Box,
            style: 0,
            key: None,
            key_atom: 0,
            text: None,
            props: vec![],
            handlers: vec![],
            children: children.into_iter().map(Child::Fresh).collect(),
            identity: 0,
            size: 0,
        }
    }

    #[test]
    fn one_keyed_child_among_unkeyed_siblings_diffs_in_place() {
        let mut enc = Encoder::default();
        let mut old = parent(vec![
            leaf(NodeKind::Text, None, "title"),
            leaf(NodeKind::Text, Some("value"), "0"),
            leaf(NodeKind::Box, None, "row"),
            leaf(NodeKind::Text, None, "hint"),
        ]);
        assign_fresh_ids(&mut enc, &mut old);
        let mut new = parent(vec![
            leaf(NodeKind::Text, None, "title"),
            leaf(NodeKind::Text, Some("value"), "1"),
            leaf(NodeKind::Box, None, "row"),
            leaf(NodeKind::Text, None, "hint"),
        ]);
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
    fn a_repeated_key_among_siblings_does_not_panic() {
        // Both new children match the one old child by key. At the second,
        // `live` has already been emptied by the first move and the target
        // index is past its end — `Vec::insert` used to panic here, and the
        // panic took the realtime worker with it.
        let mut enc = Encoder::default();
        let mut old = parent(vec![leaf(NodeKind::Box, Some("x"), "a")]);
        assign_fresh_ids(&mut enc, &mut old);
        let mut new = parent(vec![
            leaf(NodeKind::Box, Some("x"), "b"),
            leaf(NodeKind::Box, Some("x"), "c"),
        ]);
        let mut ops = Vec::new();
        diff(&mut enc, &old, &mut new, &mut ops);
        assert!(!ops.is_empty());
    }

    #[test]
    fn a_run_of_removals_is_one_op() {
        let mut enc = Encoder::default();
        let mut old = parent(
            (0..6)
                .map(|i| leaf(NodeKind::Box, Some(&i.to_string()), ""))
                .collect(),
        );
        assign_fresh_ids(&mut enc, &mut old);
        // Keep 0 and 5; 1..=4 go.
        let mut new = parent(vec![
            leaf(NodeKind::Box, Some("0"), ""),
            leaf(NodeKind::Box, Some("5"), ""),
        ]);
        let mut ops = Vec::new();
        diff(&mut enc, &old, &mut new, &mut ops);
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(matches!(
            ops[0],
            Op::RemoveChild {
                index: 1,
                count: 4,
                ..
            }
        ));
    }

    #[test]
    fn a_shuffle_replays_to_the_new_order() {
        // Apply the ops to a model of the client's child list and check it
        // ends up in the new order, for a few shuffles.
        for (old_order, new_order) in [
            (vec![0, 1, 2, 3, 4], vec![4, 3, 2, 1, 0]),
            (vec![0, 1, 2, 3, 4], vec![1, 0, 3, 2, 4]),
            (vec![0, 1, 2, 3, 4], vec![2, 4, 0, 5, 1]),
            (vec![0, 1, 2], vec![9, 2, 1, 8, 0]),
        ] {
            let mut enc = Encoder::default();
            let mut old = parent(
                old_order
                    .iter()
                    .map(|i| leaf(NodeKind::Box, Some(&i.to_string()), ""))
                    .collect(),
            );
            assign_fresh_ids(&mut enc, &mut old);
            let mut new = parent(
                new_order
                    .iter()
                    .map(|i| leaf(NodeKind::Box, Some(&i.to_string()), ""))
                    .collect(),
            );
            let mut ops = Vec::new();
            diff(&mut enc, &old, &mut new, &mut ops);
            let mut client: Vec<u32> = old.children.iter().map(|c| c.id).collect();
            for op in &ops {
                match op {
                    Op::RemoveChild { index, count, .. } => {
                        let i = *index as usize;
                        client.drain(i..i + *count as usize);
                    }
                    Op::MoveChild { from, to, .. } => {
                        let id = client.remove(*from as usize);
                        client.insert(*to as usize, id);
                    }
                    Op::InsertChild { index, subtree, .. } => {
                        client.insert(*index as usize, subtree.nodes[0].id);
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            let expected: Vec<u32> = new.children.iter().map(|c| c.id).collect();
            assert_eq!(client, expected, "{old_order:?} -> {new_order:?}: {ops:?}");
        }
    }

    #[test]
    fn a_reversed_keyed_list_is_moves() {
        let mut enc = Encoder::default();
        let mut old = parent(
            (0..5)
                .map(|i| leaf(NodeKind::Box, Some(&i.to_string()), ""))
                .collect(),
        );
        assign_fresh_ids(&mut enc, &mut old);
        let mut new = parent(
            (0..5)
                .rev()
                .map(|i| leaf(NodeKind::Box, Some(&i.to_string()), ""))
                .collect(),
        );
        let mut ops = Vec::new();
        diff(&mut enc, &old, &mut new, &mut ops);
        assert!(
            ops.iter().all(|o| matches!(o, Op::MoveChild { .. })),
            "{ops:?}"
        );
        assert_eq!(ops.len(), 4);
        assert_eq!(new.children[0].id, old.children[4].id);
    }
}
