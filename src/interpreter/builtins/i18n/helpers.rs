//! Pure i18n helper functions for use in templates.
//!
//! These functions work with primitive types and can be called from
//! both the interpreter and template contexts.

use std::cell::RefCell;

// Thread-local storage for the current locale (shared with mod.rs)
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    pub(crate) static CURRENT_LOCALE: RefCell<String> = RefCell::new("en".to_string());
}

/// Get the current locale.
pub fn get_locale() -> String {
    CURRENT_LOCALE.with(|l| l.borrow().clone())
}

/// Set the current locale.
pub fn set_locale(locale: &str) {
    CURRENT_LOCALE.with(|l| *l.borrow_mut() = locale.to_string());
}

/// Translate a key using a translations hash.
///
/// # Arguments
/// * `key` - The translation key (e.g., "hello", "messages.greeting")
/// * `locale` - The locale to use (e.g., "en", "fr")
/// * `translations` - A nested hash of translations
///
/// The translations hash should be structured like:
/// ```json
/// {
///   "en": { "hello": "Hello", "messages": { "greeting": "Hi there" } },
///   "fr": { "hello": "Bonjour", "messages": { "greeting": "Salut" } }
/// }
/// ```
pub fn translate(
    key: &str,
    locale: &str,
    translations: &[(String, TranslationValue)],
) -> Option<String> {
    // Find the locale entry
    let locale_translations = translations
        .iter()
        .find(|(k, _)| k == locale)
        .map(|(_, v)| v)?;

    // Navigate the key path (e.g., "messages.greeting" -> ["messages", "greeting"])
    let parts: Vec<&str> = key.split('.').collect();
    resolve_key(locale_translations, &parts)
}

/// A simplified translation value for the helper.
#[derive(Clone, Debug)]
pub enum TranslationValue {
    String(String),
    Hash(Vec<(String, TranslationValue)>),
}

/// Resolve a key path in a translation value.
fn resolve_key(value: &TranslationValue, parts: &[&str]) -> Option<String> {
    match (value, parts.split_first()) {
        (TranslationValue::String(s), None) => Some(s.clone()),
        (TranslationValue::String(s), Some(_)) => Some(s.clone()), // Return string even if more parts
        (TranslationValue::Hash(hash), Some((first, rest))) => {
            let next = hash.iter().find(|(k, _)| k == *first).map(|(_, v)| v)?;
            if rest.is_empty() {
                match next {
                    TranslationValue::String(s) => Some(s.clone()),
                    TranslationValue::Hash(_) => None, // Can't return a hash
                }
            } else {
                resolve_key(next, rest)
            }
        }
        (TranslationValue::Hash(_), None) => None, // Can't return a hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_set_locale() {
        set_locale("fr");
        assert_eq!(get_locale(), "fr");
        set_locale("en");
        assert_eq!(get_locale(), "en");
    }

    #[test]
    fn test_translate_simple() {
        let translations = vec![
            (
                "en".to_string(),
                TranslationValue::Hash(vec![(
                    "hello".to_string(),
                    TranslationValue::String("Hello".to_string()),
                )]),
            ),
            (
                "fr".to_string(),
                TranslationValue::Hash(vec![(
                    "hello".to_string(),
                    TranslationValue::String("Bonjour".to_string()),
                )]),
            ),
        ];

        assert_eq!(
            translate("hello", "en", &translations),
            Some("Hello".to_string())
        );
        assert_eq!(
            translate("hello", "fr", &translations),
            Some("Bonjour".to_string())
        );
    }

    #[test]
    fn test_translate_nested() {
        let translations = vec![(
            "en".to_string(),
            TranslationValue::Hash(vec![(
                "messages".to_string(),
                TranslationValue::Hash(vec![(
                    "greeting".to_string(),
                    TranslationValue::String("Hi there".to_string()),
                )]),
            )]),
        )];

        assert_eq!(
            translate("messages.greeting", "en", &translations),
            Some("Hi there".to_string())
        );
    }
}
