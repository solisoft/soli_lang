//! Hook system for Model lifecycle events.

#[derive(Debug, Clone, PartialEq)]
pub enum HookType {
    BeforeCreate,
    AfterCreate,
    BeforeSave,
    AfterSave,
    BeforeUpdate,
    AfterUpdate,
    BeforeDelete,
    AfterDelete,
    BeforeValidate,
    AfterValidate,
    BeforeFind,
    AfterFind,
}

impl HookType {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookType::BeforeCreate => "before_create",
            HookType::AfterCreate => "after_create",
            HookType::BeforeSave => "before_save",
            HookType::AfterSave => "after_save",
            HookType::BeforeUpdate => "before_update",
            HookType::AfterUpdate => "after_update",
            HookType::BeforeDelete => "before_delete",
            HookType::AfterDelete => "after_delete",
            HookType::BeforeValidate => "before_validate",
            HookType::AfterValidate => "after_validate",
            HookType::BeforeFind => "before_find",
            HookType::AfterFind => "after_find",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hook {
    pub hook_type: HookType,
    pub name: String,
}

impl Hook {
    pub fn new(hook_type: HookType, name: &str) -> Self {
        Self {
            hook_type,
            name: name.to_string(),
        }
    }
}

#[derive(Debug, Default)]
pub struct HookStore {
    hooks: Vec<Hook>,
}

impl HookStore {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook_type: HookType, name: &str) {
        self.hooks.push(Hook::new(hook_type, name));
    }

    pub fn has_hooks(&self, hook_type: &HookType) -> bool {
        self.hooks.iter().any(|h| h.hook_type == *hook_type)
    }
}
