//! Lifecycle callbacks for models.

use std::cell::RefCell;

use super::core::MODEL_REGISTRY;
use super::validation::ValidationRule;

/// Lifecycle callbacks for a model.
#[derive(Debug, Clone, Default)]
pub struct ModelCallbacks {
    pub before_save: Vec<String>,
    pub after_save: Vec<String>,
    pub before_create: Vec<String>,
    pub after_create: Vec<String>,
    pub before_update: Vec<String>,
    pub after_update: Vec<String>,
    pub before_delete: Vec<String>,
    pub after_delete: Vec<String>,
}

impl ModelCallbacks {
    /// The list of one kind, `None` for a kind that does not exist.
    fn list_mut(&mut self, callback_type: &str) -> Option<&mut Vec<String>> {
        Some(match callback_type {
            "before_save" => &mut self.before_save,
            "after_save" => &mut self.after_save,
            "before_create" => &mut self.before_create,
            "after_create" => &mut self.after_create,
            "before_update" => &mut self.before_update,
            "after_update" => &mut self.after_update,
            "before_delete" => &mut self.before_delete,
            "after_delete" => &mut self.after_delete,
            _ => return None,
        })
    }
}

thread_local! {
    /// The callbacks of the model classes being declared on this thread —
    /// a stack, as a model class can declare another inside it. See
    /// [`Declaration`].
    static DECLARING: RefCell<Vec<Declaring>> = const { RefCell::new(Vec::new()) };
}

/// What one model class body has declared so far.
struct Declaring {
    class_name: String,
    callbacks: ModelCallbacks,
    validations: Vec<ValidationRule>,
}

/// Add `rule` to the validations `class_name`'s body is building, when its
/// body is running on this thread ([`Declaration`]); `false` otherwise, and
/// the caller registers it directly. An identical rule is kept once.
pub(super) fn declare_validation(class_name: &str, rule: &ValidationRule) -> bool {
    DECLARING.with(|stack| {
        let mut stack = stack.borrow_mut();
        let Some(declaring) = stack.iter_mut().rev().find(|d| d.class_name == class_name) else {
            return false;
        };
        if !declaring.validations.iter().any(|r| r == rule) {
            declaring.validations.push(rule.clone());
        }
        true
    })
}

/// Register a callback for a model class.
///
/// In declaration order, and a name declared twice runs twice. While the
/// class's body runs, this adds to the list the body is building
/// ([`Declaration`]); otherwise to the class's registered list.
pub fn register_callback(class_name: &str, callback_type: &str, method_name: &str) {
    let declaring = DECLARING.with(|stack| {
        let mut stack = stack.borrow_mut();
        let Some(declaring) = stack.iter_mut().rev().find(|d| d.class_name == class_name) else {
            return false;
        };
        if let Some(list) = declaring.callbacks.list_mut(callback_type) {
            list.push(method_name.to_string());
        }
        true
    });
    if declaring {
        return;
    }
    MODEL_REGISTRY.write(|registry| {
        let metadata = registry.entry(class_name.to_string()).or_default();
        if let Some(list) = metadata.callbacks.list_mut(callback_type) {
            list.push(method_name.to_string());
        }
    })
}

/// The callbacks a model class's body declares, installed as a whole when
/// the body has run.
///
/// The body runs again on every hot reload, in every worker, and the
/// registry is process-wide. Appending to it straight away added each
/// callback again each time: after a day of edits, one `save` ran its
/// `before_save` thousands of times. Built aside and swapped in, the list is
/// the body's, whichever worker installs it last, and never seen half-built.
pub struct Declaration {
    class_name: String,
    installed: bool,
}

impl Declaration {
    /// Start collecting `class_name`'s callbacks and validation rules, from
    /// `inherited` — the parent's for a model that inherits from another,
    /// empty otherwise.
    ///
    /// Validations are built aside like callbacks: they too were added to the
    /// registry as the body ran, so an edited `validates(...)` (a different
    /// rule, `on: "create"` added) joined the old one on a `--dev` reload
    /// instead of replacing it, and the old rule kept refusing saves.
    pub fn begin(
        class_name: &str,
        inherited: ModelCallbacks,
        inherited_validations: Vec<ValidationRule>,
    ) -> Self {
        DECLARING.with(|stack| {
            stack.borrow_mut().push(Declaring {
                class_name: class_name.to_string(),
                callbacks: inherited,
                validations: inherited_validations,
            })
        });
        Declaration {
            class_name: class_name.to_string(),
            installed: false,
        }
    }

    /// The body has run: its callbacks and validations replace the class's.
    pub fn install(mut self) {
        if let Some(declared) = self.take() {
            MODEL_REGISTRY.write(|registry| {
                let metadata = registry.entry(self.class_name.clone()).or_default();
                metadata.callbacks = declared.callbacks;
                metadata.validations = declared.validations;
            });
        }
        self.installed = true;
    }

    fn take(&self) -> Option<Declaring> {
        DECLARING.with(|stack| {
            let mut stack = stack.borrow_mut();
            let at = stack
                .iter()
                .rposition(|d| d.class_name == self.class_name)?;
            Some(stack.remove(at))
        })
    }
}

impl Drop for Declaration {
    /// A body that failed half-way leaves the class's callbacks as they were.
    fn drop(&mut self) {
        if !self.installed {
            self.take();
        }
    }
}

/// The validation rules registered for `class_name`, empty when it has none.
pub fn registered_validations(class_name: &str) -> Vec<ValidationRule> {
    MODEL_REGISTRY.read(|registry| {
        registry
            .get(class_name)
            .map(|meta| meta.validations.clone())
            .unwrap_or_default()
    })
}

/// The callbacks registered for `class_name`, empty when it has none.
pub fn registered(class_name: &str) -> ModelCallbacks {
    MODEL_REGISTRY.read(|registry| {
        registry
            .get(class_name)
            .map(|meta| meta.callbacks.clone())
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A class body run again — a hot reload, in each worker — replaces its
    /// callbacks; within one run, a name declared twice is kept twice.
    #[test]
    fn a_class_body_run_again_replaces_its_callbacks() {
        let class = "CallbackDeclarationProbe";
        for _ in 0..3 {
            let declaration = Declaration::begin(class, ModelCallbacks::default(), Vec::new());
            register_callback(class, "before_save", "normalize");
            register_callback(class, "before_save", "normalize");
            register_callback(class, "after_create", "announce");
            declaration.install();
        }
        let callbacks = registered(class);
        assert_eq!(callbacks.before_save, vec!["normalize", "normalize"]);
        assert_eq!(callbacks.after_create, vec!["announce"]);

        // A body that fails half-way changes nothing.
        let declaration = Declaration::begin(class, ModelCallbacks::default(), Vec::new());
        register_callback(class, "before_save", "half");
        drop(declaration);
        assert_eq!(
            registered(class).before_save,
            vec!["normalize", "normalize"]
        );

        // Outside a declaration, a callback is added to the registered list.
        register_callback(class, "after_create", "late");
        assert_eq!(registered(class).after_create, vec!["announce", "late"]);
    }

    /// An edited `validates(...)` replaces the rule it was, on a reload: the
    /// old rule (no `on:`) stayed beside the new one (`on: "create"`) and kept
    /// refusing updates until the server restarted.
    #[test]
    fn a_class_body_run_again_replaces_its_validations() {
        use super::super::validation::register_validation;
        let class = "ValidationDeclarationProbe";
        let rule = |on: Option<&str>| ValidationRule {
            field: "name".into(),
            custom: Some("only_on_create".into()),
            on: on.map(str::to_string),
            ..Default::default()
        };

        let declaration = Declaration::begin(class, ModelCallbacks::default(), Vec::new());
        register_validation(class, rule(None));
        declaration.install();
        assert_eq!(registered_validations(class), vec![rule(None)]);

        // The edited body, run again.
        let declaration = Declaration::begin(class, ModelCallbacks::default(), Vec::new());
        register_validation(class, rule(Some("create")));
        declaration.install();
        assert_eq!(registered_validations(class), vec![rule(Some("create"))]);

        // A body that fails half-way changes nothing.
        let declaration = Declaration::begin(class, ModelCallbacks::default(), Vec::new());
        register_validation(class, rule(None));
        drop(declaration);
        assert_eq!(registered_validations(class), vec![rule(Some("create"))]);
    }
}
