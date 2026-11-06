//! What each `CallMethod` / `CallMethodById` site found on each receiver
//! class it saw: the compiled method and the class that defines it, when the
//! nearest definition of the name (`Class::find_instance_method_level`) is
//! compiled. The fast tier's method calls (`Vm::fast_method`) ask here
//! instead of walking the class chain and hashing the name at every level on
//! every call.
//!
//! Nearest, not first compiled: a subclass override the VM has not compiled
//! yet hides an ancestor's compiled method of the same name. Taking the
//! first compiled method anywhere up the chain ran `Base#m` for a `Leaf`
//! whose own `m` was still AST, once `Base#m` had run on that worker. Such a
//! lookup is a miss here: the general arm compiles the override
//! (`Vm::resolve_instance_method`), which bumps the epoch.
//!
//! An entry answers for one call site (the op's address, which fixes the
//! name) and one receiver class (which fixes the chain), in one
//! method-definition epoch (`value::vm_methods_epoch`, bumped by every change
//! to some class's `vm_methods` or `methods`). Those three fix what the
//! uncached lookup returns, so a current entry answers exactly what it
//! would. A miss is remembered too: a native method's site then hands its
//! call to the general arm after one probe.
//!
//! An entry keeps its class and the code holding its site alive, so neither
//! address can be reused by another class or op while the entry exists.

use std::rc::Rc;
use std::sync::Arc;

use crate::interpreter::value::{Class, InstanceMethodDef};
use crate::vm::chunk::FunctionProto;
use crate::vm::opcode::Op;
use crate::vm::upvalue::VmClosure;

/// A compiled method and the class that defines it, its frame's class.
pub(crate) type FoundMethod = (Rc<VmClosure>, Rc<Class>);

/// log2 of the number of sets.
const SET_BITS: u32 = 7;
/// Sets in the cache.
const SETS: usize = 1 << SET_BITS;
/// Entries per set.
const WAYS: usize = 2;

struct Entry {
    site: *const Op,
    class: Rc<Class>,
    epoch: u64,
    found: Option<FoundMethod>,
    /// The code `site` points into, kept alive (see the module docs).
    _code: Arc<FunctionProto>,
}

/// The per-VM method cache: `SETS` sets of `WAYS` entries, allocated on the
/// first fill.
#[derive(Default)]
pub(crate) struct MethodCache {
    sets: Option<Box<[[Option<Entry>; WAYS]; SETS]>>,
}

/// The set for `site` and `class`: both addresses, so one polymorphic site
/// spreads its classes over several sets.
#[inline]
fn set_of(site: *const Op, class: &Rc<Class>) -> usize {
    let key = (site as usize) ^ (Rc::as_ptr(class) as usize).rotate_left(17);
    // Fibonacci hashing: the product's top bits mix every bit of the key.
    (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as usize >> (usize::BITS - SET_BITS)
}

impl MethodCache {
    /// The compiled nearest definition of `name` on `class` (see the module
    /// docs), for the call at
    /// `site` (whose op names `name`) in `epoch`. `code` gives the code that
    /// holds `site`, asked only on a fill.
    #[inline]
    pub(crate) fn lookup(
        &mut self,
        site: *const Op,
        class: &Rc<Class>,
        name: &str,
        epoch: u64,
        code: impl FnOnce() -> Arc<FunctionProto>,
    ) -> Option<FoundMethod> {
        if let Some(sets) = &self.sets {
            for entry in sets[set_of(site, class)].iter().flatten() {
                if entry.site == site && Rc::ptr_eq(&entry.class, class) && entry.epoch == epoch {
                    return entry.found.clone();
                }
            }
        }
        self.fill(site, class, name, epoch, code())
    }

    /// Look `name` up on `class` and remember the answer. A stale or empty
    /// first way is overwritten; otherwise the first way moves to the second.
    #[cold]
    #[inline(never)]
    fn fill(
        &mut self,
        site: *const Op,
        class: &Rc<Class>,
        name: &str,
        epoch: u64,
        code: Arc<FunctionProto>,
    ) -> Option<FoundMethod> {
        let found = match class.find_instance_method_level(name) {
            Some((InstanceMethodDef::Compiled(closure), defining)) => Some((closure, defining)),
            // Not compiled yet, or no such method: the general arm decides.
            _ => None,
        };
        let sets = self
            .sets
            .get_or_insert_with(|| Box::new(std::array::from_fn(|_| [None, None])));
        let set = &mut sets[set_of(site, class)];
        let first_is_current = set[0].as_ref().is_some_and(|entry| entry.epoch == epoch);
        if first_is_current {
            set[1] = set[0].take();
        }
        set[0] = Some(Entry {
            site,
            class: class.clone(),
            epoch,
            found: found.clone(),
            _code: code,
        });
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::vm_methods_epoch;

    fn proto() -> Arc<FunctionProto> {
        Arc::new(FunctionProto::new("site".to_string()))
    }

    fn class(name: &str, superclass: Option<Rc<Class>>) -> Rc<Class> {
        Rc::new(Class {
            name: name.to_string(),
            superclass,
            ..Default::default()
        })
    }

    fn method() -> Rc<VmClosure> {
        Rc::new(VmClosure::new(proto(), Vec::new()))
    }

    /// The first op of `code`: a site whose address `code` keeps.
    fn site_of(code: &Arc<FunctionProto>) -> *const Op {
        code.chunk.code.as_ptr()
    }

    fn proto_with_ops(n: usize) -> Arc<FunctionProto> {
        let mut proto = FunctionProto::new("site".to_string());
        for _ in 0..n {
            proto.chunk.code.push(Op::Nop);
        }
        Arc::new(proto)
    }

    /// The answer of a lookup, as (closure address, defining class name).
    fn shape(found: Option<FoundMethod>) -> Option<(*const VmClosure, String)> {
        found.map(|(closure, defining)| (Rc::as_ptr(&closure), defining.name.clone()))
    }

    /// A first lookup walks the chain; a second at the same site, class and
    /// epoch answers from the entry, without reading the map again: here the
    /// map is edited behind the cache's back, with no epoch change, and the
    /// entry still answers what it found.
    #[test]
    fn a_current_entry_answers_without_a_lookup() {
        let base = class("Base", None);
        let derived = class("Derived", Some(base.clone()));
        let greet = method();
        base.vm_methods
            .borrow_mut()
            .insert("greet".into(), greet.clone());
        let code = proto_with_ops(1);
        let site = site_of(&code);
        let mut cache = MethodCache::default();
        let first = cache.lookup(site, &derived, "greet", 7, || code.clone());
        assert_eq!(shape(first), Some((Rc::as_ptr(&greet), "Base".to_string())));
        derived
            .vm_methods
            .borrow_mut()
            .insert("greet".into(), method());
        let again = cache.lookup(site, &derived, "greet", 7, || panic!("no fill"));
        assert_eq!(shape(again), Some((Rc::as_ptr(&greet), "Base".to_string())));
    }

    /// A new epoch refills: the entry found the parent's method, the class
    /// now has its own.
    #[test]
    fn a_new_epoch_refills() {
        let base = class("Base", None);
        let derived = class("Derived", Some(base.clone()));
        base.vm_methods
            .borrow_mut()
            .insert("greet".into(), method());
        let code = proto_with_ops(1);
        let site = site_of(&code);
        let mut cache = MethodCache::default();
        cache.lookup(site, &derived, "greet", 1, || code.clone());
        let own = method();
        derived
            .vm_methods
            .borrow_mut()
            .insert("greet".into(), own.clone());
        let found = cache.lookup(site, &derived, "greet", 2, || code.clone());
        assert_eq!(
            shape(found),
            Some((Rc::as_ptr(&own), "Derived".to_string()))
        );
    }

    /// One site seeing two classes keeps an answer for each, and a miss is
    /// remembered and staled like a hit.
    #[test]
    fn one_site_keeps_each_class_and_remembers_a_miss() {
        let circle = class("Circle", None);
        let square = class("Square", None);
        let blank = class("Blank", None);
        let (round, flat) = (method(), method());
        circle
            .vm_methods
            .borrow_mut()
            .insert("area".into(), round.clone());
        square
            .vm_methods
            .borrow_mut()
            .insert("area".into(), flat.clone());
        let code = proto_with_ops(1);
        let site = site_of(&code);
        let mut cache = MethodCache::default();
        for _ in 0..3 {
            let a = cache.lookup(site, &circle, "area", 3, || code.clone());
            let b = cache.lookup(site, &square, "area", 3, || code.clone());
            assert_eq!(shape(a).unwrap().0, Rc::as_ptr(&round));
            assert_eq!(shape(b).unwrap().0, Rc::as_ptr(&flat));
        }
        assert!(cache
            .lookup(site, &blank, "area", 3, || code.clone())
            .is_none());
        blank
            .vm_methods
            .borrow_mut()
            .insert("area".into(), method());
        // Same epoch: the remembered miss answers.
        assert!(cache
            .lookup(site, &blank, "area", 3, || panic!("no fill"))
            .is_none());
        // A new one: the method is found.
        assert!(cache
            .lookup(site, &blank, "area", 4, || code.clone())
            .is_some());
    }

    /// An entry keeps its class and its site's code alive, so their
    /// addresses cannot come back as another class's or op's.
    #[test]
    fn an_entry_keeps_its_class_and_code_alive() {
        let shape_class = class("Shape", None);
        let code = proto_with_ops(1);
        let site = site_of(&code);
        let mut cache = MethodCache::default();
        cache.lookup(site, &shape_class, "area", 1, || code.clone());
        assert_eq!(Rc::strong_count(&shape_class), 2);
        assert_eq!(Arc::strong_count(&code), 2);
        drop(cache);
        assert_eq!(Rc::strong_count(&shape_class), 1);
        assert_eq!(Arc::strong_count(&code), 1);
    }

    /// Many sites in one cache: every lookup still answers what the chain
    /// gives, whatever was evicted on the way.
    #[test]
    fn a_crowded_cache_still_answers_right() {
        let classes: Vec<Rc<Class>> = (0..8)
            .map(|i| {
                let c = class(&format!("C{i}"), None);
                if i % 2 == 0 {
                    c.vm_methods.borrow_mut().insert("m".into(), method());
                }
                c
            })
            .collect();
        let code = proto_with_ops(600);
        let mut cache = MethodCache::default();
        for round in 0..3 {
            for i in 0..600 {
                let site = code.chunk.code[i..].as_ptr();
                for (k, c) in classes.iter().enumerate() {
                    let found = cache.lookup(site, c, "m", 9, || code.clone());
                    assert_eq!(found.is_some(), k % 2 == 0, "round {round}, site {i}, C{k}");
                }
            }
        }
    }

    /// Every way a class's `vm_methods` changes moves the epoch:
    /// `insert_vm_method` and `include_module`. (Other test threads move it
    /// too, so only "it moved" is asserted.)
    #[test]
    fn changing_vm_methods_moves_the_epoch() {
        let before = vm_methods_epoch();
        class("A", None).insert_vm_method("m".into(), method());
        let after_insert = vm_methods_epoch();
        assert!(after_insert > before);
        let module = Rc::new(Class {
            name: "Mixin".to_string(),
            is_module: true,
            ..Default::default()
        });
        module.vm_methods.borrow_mut().insert("n".into(), method());
        class("B", None).include_module(&module).expect("include");
        assert!(vm_methods_epoch() > after_insert);
    }
}
