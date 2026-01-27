//! Hidden Classes (Shapes) for objects - enables inline caching and fast property access.

use crate::interpreter::{HiddenClassId, SymbolId, Value, INLINE_CACHE};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::RwLock;

lazy_static! {
    pub static ref HIDDEN_CLASS_REGISTRY: HiddenClassRegistry = HiddenClassRegistry::new();
}

#[derive(Debug, Clone)]
pub struct HiddenClass {
    pub id: HiddenClassId,
    pub parent_id: Option<HiddenClassId>,
    pub property_offsets: HashMap<SymbolId, usize>,
    pub property_count: usize,
    pub transition_map: HashMap<SymbolId, HiddenClassId>,
    pub is_sealed: bool,
}

impl HiddenClass {
    pub fn new(id: HiddenClassId, parent_id: Option<HiddenClassId>) -> Self {
        Self {
            id,
            parent_id,
            property_offsets: HashMap::new(),
            property_count: 0,
            transition_map: HashMap::new(),
            is_sealed: false,
        }
    }

    pub fn get_property_offset(&self, symbol_id: SymbolId) -> Option<usize> {
        if let Some(offset) = self.property_offsets.get(&symbol_id) {
            Some(*offset)
        } else if let Some(parent_id) = self.parent_id {
            HIDDEN_CLASS_REGISTRY
                .get(parent_id)
                .and_then(|hc| hc.get_property_offset(symbol_id))
        } else {
            None
        }
    }

    pub fn has_property(&self, symbol_id: SymbolId) -> bool {
        self.property_offsets.contains_key(&symbol_id)
            || self
                .parent_id
                .and_then(|pid| {
                    HIDDEN_CLASS_REGISTRY
                        .get(pid)
                        .map(|hc| hc.has_property(symbol_id))
                })
                .unwrap_or(false)
    }

    pub fn total_property_count(&self) -> usize {
        let parent_count = self
            .parent_id
            .and_then(|pid| HIDDEN_CLASS_REGISTRY.get(pid))
            .map(|hc| hc.total_property_count())
            .unwrap_or(0);
        self.property_count + parent_count
    }
}

#[derive(Debug)]
pub struct HiddenClassRegistry {
    classes: RwLock<HashMap<HiddenClassId, HiddenClass>>,
    transitions: RwLock<HashMap<(HiddenClassId, SymbolId), HiddenClassId>>,
    root_id: HiddenClassId,
}

impl HiddenClassRegistry {
    fn new() -> Self {
        let root_id = HiddenClassId(0);
        let mut classes = HashMap::new();
        classes.insert(root_id, HiddenClass::new(root_id, None));
        Self {
            classes: RwLock::new(classes),
            transitions: RwLock::new(HashMap::new()),
            root_id,
        }
    }

    pub fn get(&self, id: HiddenClassId) -> Option<HiddenClass> {
        self.classes.read().unwrap().get(&id).cloned()
    }

    pub fn root(&self) -> HiddenClassId {
        self.root_id
    }

    pub fn add_property(
        &self,
        current_id: HiddenClassId,
        symbol_id: SymbolId,
    ) -> (HiddenClassId, usize) {
        {
            let transitions = self.transitions.read().unwrap();
            if let Some(&new_id) = transitions.get(&(current_id, symbol_id)) {
                let new_class = self.classes.read().unwrap().get(&new_id).cloned().unwrap();
                let offset = new_class.property_offsets.get(&symbol_id).copied().unwrap();
                return (new_id, offset);
            }
        }

        let current_class = self
            .classes
            .read()
            .unwrap()
            .get(&current_id)
            .cloned()
            .unwrap();
        if current_class.is_sealed {
            return (current_id, current_class.total_property_count());
        }

        let new_id = INLINE_CACHE.new_hidden_class_id();
        let new_offset = current_class.total_property_count();

        let mut new_class = HiddenClass::new(new_id, Some(current_id));
        new_class.property_offsets = current_class.property_offsets.clone();
        new_class.property_offsets.insert(symbol_id, new_offset);
        new_class.property_count = current_class.property_count + 1;
        new_class.transition_map = current_class.transition_map.clone();

        {
            let mut classes = self.classes.write().unwrap();
            classes.insert(new_id, new_class.clone());
        }

        {
            let mut transitions = self.transitions.write().unwrap();
            transitions.insert((current_id, symbol_id), new_id);
        }

        (new_id, new_offset)
    }

    pub fn seal(&self, id: HiddenClassId) {
        if let Some(class) = self.classes.write().unwrap().get_mut(&id) {
            class.is_sealed = true;
        }
    }

    pub fn own_properties(&self, id: HiddenClassId) -> Vec<(SymbolId, usize)> {
        if let Some(class) = self.classes.read().unwrap().get(&id) {
            class
                .property_offsets
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect()
        } else {
            vec![]
        }
    }
}

#[derive(Debug, Clone)]
pub struct HiddenClassObject {
    pub hidden_class_id: HiddenClassId,
    pub fields: Vec<(SymbolId, Value)>,
}

impl HiddenClassObject {
    pub fn new() -> Self {
        Self {
            hidden_class_id: HIDDEN_CLASS_REGISTRY.root(),
            fields: Vec::new(),
        }
    }

    pub fn get(&self, symbol_id: SymbolId) -> Option<&Value> {
        if let Some(offset) = HIDDEN_CLASS_REGISTRY
            .get(self.hidden_class_id)
            .and_then(|hc| hc.get_property_offset(symbol_id))
        {
            if offset < self.fields.len() {
                return self.fields.get(offset).and_then(|(s, v)| {
                    if *s == symbol_id {
                        Some(v)
                    } else {
                        None
                    }
                });
            }
        }
        self.fields
            .iter()
            .find(|(s, _)| *s == symbol_id)
            .map(|(_, v)| v)
    }

    pub fn set(&mut self, symbol_id: SymbolId, value: Value) {
        if let Some(offset) = HIDDEN_CLASS_REGISTRY
            .get(self.hidden_class_id)
            .and_then(|hc| hc.get_property_offset(symbol_id))
        {
            if offset < self.fields.len() {
                if self.fields[offset].0 == symbol_id {
                    self.fields[offset].1 = value;
                    return;
                }
            }
        }

        let (new_id, new_offset) =
            HIDDEN_CLASS_REGISTRY.add_property(self.hidden_class_id, symbol_id);
        self.hidden_class_id = new_id;

        while self.fields.len() <= new_offset {
            self.fields.push((symbol_id, Value::Null));
        }

        self.fields[new_offset] = (symbol_id, value);
    }

    pub fn hidden_class_id(&self) -> HiddenClassId {
        self.hidden_class_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::get_symbol;

    #[test]
    fn test_hidden_class_transitions() {
        let registry = HiddenClassRegistry::new();

        let root = registry.root();
        let (id1, offset1) = registry.add_property(root, get_symbol("a"));
        assert_eq!(offset1, 0);

        let (id2, offset2) = registry.add_property(id1, get_symbol("b"));
        assert_eq!(offset2, 1);

        let (id2_again, _) = registry.add_property(id1, get_symbol("b"));
        assert_eq!(id2_again, id2);
    }

    #[test]
    fn test_hidden_class_object() {
        let mut obj = HiddenClassObject::new();

        obj.set(get_symbol("name"), Value::String("Alice".to_string()));
        obj.set(get_symbol("age"), Value::Float(30.0));

        assert_eq!(obj.get(get_symbol("name")).unwrap().to_string(), "Alice");
    }

    #[test]
    fn test_hidden_class_property_lookup() {
        let registry = HiddenClassRegistry::new();
        let (id, _) = registry.add_property(registry.root(), get_symbol("test"));

        let class = registry.get(id).unwrap();
        assert!(class.has_property(get_symbol("test")));
        assert_eq!(class.get_property_offset(get_symbol("test")), Some(0));
    }
}
