//! Pure i18n helper functions for use in templates.
//!
//! These functions work with primitive types and can be called from
//! both the interpreter and template contexts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

// Thread-local storage for the current locale (shared with mod.rs)
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    pub(crate) static CURRENT_LOCALE: RefCell<String> = RefCell::new("en".to_string());
}

/// Process-wide store of translations loaded from `config/locales/*.yml` at boot.
/// Keyed by locale name (top-level YAML key, e.g. `"en"`); values are the parsed
/// YAML subtree under that key. `serde_yaml::Value` is `Send + Sync`, so a single
/// global serves all worker threads.
static TRANSLATIONS: RwLock<Option<HashMap<String, serde_yaml::Value>>> = RwLock::new(None);

/// Get the current locale.
pub fn get_locale() -> String {
    CURRENT_LOCALE.with(|l| l.borrow().clone())
}

/// Set the current locale.
pub fn set_locale(locale: &str) {
    CURRENT_LOCALE.with(|l| *l.borrow_mut() = locale.to_string());
}

/// Load all `*.yml` / `*.yaml` files under `<config_dir>/locales/` into the
/// global translations store. The top-level YAML node of each file must be a
/// mapping; each `(locale, subtree)` pair is merged into the store, so a single
/// file may declare multiple locales and multiple files may extend the same
/// locale (Rails convention).
///
/// Returns the number of locales loaded. A missing `locales/` dir is a no-op
/// (returns 0). A malformed file logs a warning to stderr and is skipped.
pub fn load_locales_from_config_dir(config_dir: &Path) -> usize {
    let locales_dir = config_dir.join("locales");
    let mut store: HashMap<String, serde_yaml::Value> = HashMap::new();

    if locales_dir.is_dir() {
        match std::fs::read_dir(&locales_dir) {
            Ok(entries) => {
                let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
                paths.sort();
                for path in paths {
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    if ext != "yml" && ext != "yaml" {
                        continue;
                    }
                    let body = match std::fs::read_to_string(&path) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("Warning: failed to read {}: {}", path.display(), e);
                            continue;
                        }
                    };
                    let parsed: serde_yaml::Value = match serde_yaml::from_str(&body) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("Warning: invalid YAML in {}: {}", path.display(), e);
                            continue;
                        }
                    };
                    let mapping = match parsed.as_mapping() {
                        Some(m) => m,
                        None => {
                            eprintln!(
                                "Warning: {} top-level YAML node is not a mapping; skipping",
                                path.display()
                            );
                            continue;
                        }
                    };
                    for (k, v) in mapping {
                        let locale = match k.as_str() {
                            Some(s) => s.to_string(),
                            None => continue,
                        };
                        let entry = store
                            .entry(locale)
                            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
                        merge_yaml(entry, v.clone());
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: failed to read {}: {}", locales_dir.display(), e);
            }
        }
    }

    let count = store.len();
    *TRANSLATIONS.write().unwrap() = Some(store);
    count
}

/// Recursively merge `src` into `dst`. Mappings are merged key-by-key; any
/// other YAML kind in `src` overwrites `dst`.
fn merge_yaml(dst: &mut serde_yaml::Value, src: serde_yaml::Value) {
    match (dst, src) {
        (serde_yaml::Value::Mapping(d), serde_yaml::Value::Mapping(s)) => {
            for (k, v) in s {
                if let Some(existing) = d.get_mut(&k) {
                    merge_yaml(existing, v);
                } else {
                    d.insert(k, v);
                }
            }
        }
        (slot, other) => {
            *slot = other;
        }
    }
}

/// Look up a dotted key in the given locale. Falls back to `"en"` if the
/// active locale has no entry. Returns `None` if no locale resolves or the
/// terminal node is not a string.
pub fn lookup_translation(locale: &str, key: &str) -> Option<String> {
    let guard = TRANSLATIONS.read().unwrap();
    let store = guard.as_ref()?;
    if let Some(s) = lookup_in(store, locale, key) {
        return Some(s);
    }
    if locale != "en" {
        return lookup_in(store, "en", key);
    }
    None
}

/// Look up a pluralized key (`<key>_zero` for n==0, `<key>_one` for n==1,
/// `<key>_other` otherwise). Falls back to `"en"`.
pub fn lookup_plural(locale: &str, key: &str, n: i64) -> Option<String> {
    let suffix = if n == 0 {
        "_zero"
    } else if n == 1 {
        "_one"
    } else {
        "_other"
    };
    let plural_key = format!("{}{}", key, suffix);
    lookup_translation(locale, &plural_key)
}

fn lookup_in(
    store: &HashMap<String, serde_yaml::Value>,
    locale: &str,
    key: &str,
) -> Option<String> {
    let mut node = store.get(locale)?;
    for part in key.split('.') {
        node = node
            .as_mapping()?
            .get(serde_yaml::Value::String(part.to_string()))?;
    }
    node.as_str().map(|s| s.to_string())
}

/// Substitute `{name}` placeholders in `template` with stringified values from
/// `values` (a slice of `(name, replacement)` pairs). Unknown placeholders are
/// left intact, so missing data is visible during development.
pub fn interpolate(template: &str, values: &[(String, String)]) -> String {
    if values.is_empty() || !template.contains('{') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find('}') {
            let name = &after_open[..close];
            if !name.is_empty() && !name.contains('{') {
                if let Some(val) = values.iter().find(|(k, _)| k == name).map(|(_, v)| v) {
                    out.push_str(val);
                    rest = &after_open[close + 1..];
                    continue;
                }
            }
        }
        out.push('{');
        rest = after_open;
    }
    out.push_str(rest);
    out
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

    // --- Tests for the new YAML-backed store -------------------------------

    use std::fs;
    use std::sync::Mutex;

    // Tests touch a process-wide static; serialize them to avoid cross-test
    // contamination.
    static GUARD: Mutex<()> = Mutex::new(());

    fn write_yaml(dir: &std::path::Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn store_loads_basic_yaml_and_resolves_dotted_keys() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  app:\n    welcome: Welcome\n  greeting: Hello\n",
        );
        write_yaml(
            &locales,
            "fr.yml",
            "fr:\n  app:\n    welcome: Bienvenue\n  greeting: Bonjour\n",
        );
        let count = load_locales_from_config_dir(&cfg);
        assert_eq!(count, 2);
        assert_eq!(
            lookup_translation("en", "app.welcome"),
            Some("Welcome".to_string())
        );
        assert_eq!(
            lookup_translation("fr", "app.welcome"),
            Some("Bienvenue".to_string())
        );
        assert_eq!(
            lookup_translation("fr", "greeting"),
            Some("Bonjour".to_string())
        );
    }

    #[test]
    fn store_falls_back_to_en_when_active_locale_misses() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  greeting: Hello\n  only_en: yes\n",
        );
        write_yaml(&locales, "fr.yml", "fr:\n  greeting: Bonjour\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("fr", "only_en"), Some("yes".to_string()));
        assert_eq!(lookup_translation("fr", "missing"), None);
    }

    #[test]
    fn store_skips_invalid_yaml_files() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        // Tab indentation is invalid in YAML; serde_yaml will reject this.
        write_yaml(&locales, "broken.yml", "en:\n\tnope: x\n");
        write_yaml(&locales, "good.yml", "en:\n  ok: yes\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "ok"), Some("yes".to_string()));
    }

    #[test]
    fn store_no_op_when_locales_dir_missing() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let count = load_locales_from_config_dir(tmp.path());
        assert_eq!(count, 0);
        assert_eq!(lookup_translation("en", "anything"), None);
    }

    #[test]
    fn store_merges_multiple_files_into_same_locale() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(&locales, "core.yml", "en:\n  app:\n    welcome: Welcome\n");
        write_yaml(
            &locales,
            "accounts.yml",
            "en:\n  accounts:\n    title: Accounts\n",
        );
        load_locales_from_config_dir(&cfg);
        assert_eq!(
            lookup_translation("en", "app.welcome"),
            Some("Welcome".to_string())
        );
        assert_eq!(
            lookup_translation("en", "accounts.title"),
            Some("Accounts".to_string())
        );
    }

    #[test]
    fn store_handles_multi_locale_single_file() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "all.yml",
            "en:\n  hi: Hello\nfr:\n  hi: Bonjour\nde:\n  hi: Hallo\n",
        );
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), Some("Hello".to_string()));
        assert_eq!(lookup_translation("fr", "hi"), Some("Bonjour".to_string()));
        assert_eq!(lookup_translation("de", "hi"), Some("Hallo".to_string()));
    }

    #[test]
    fn plural_lookup_picks_correct_suffix() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  items_zero: No items\n  items_one: One item\n  items_other: \"{count} items\"\n",
        );
        load_locales_from_config_dir(&cfg);
        assert_eq!(
            lookup_plural("en", "items", 0),
            Some("No items".to_string())
        );
        assert_eq!(
            lookup_plural("en", "items", 1),
            Some("One item".to_string())
        );
        assert_eq!(
            lookup_plural("en", "items", 5),
            Some("{count} items".to_string())
        );
    }

    #[test]
    fn interpolate_basic_and_missing_keys() {
        let v = vec![("name".to_string(), "Alice".to_string())];
        assert_eq!(interpolate("Welcome, {name}!", &v), "Welcome, Alice!");
        assert_eq!(
            interpolate("Hello {missing}", &v),
            "Hello {missing}".to_string()
        );
        assert_eq!(interpolate("no placeholders", &v), "no placeholders");
    }

    #[test]
    fn interpolate_preserves_unicode() {
        let v = vec![("name".to_string(), "Élise".to_string())];
        assert_eq!(
            interpolate("Bienvenue, {name} — bonne journée!", &v),
            "Bienvenue, Élise — bonne journée!"
        );
    }

    #[test]
    fn interpolate_handles_repeats_and_empty_values() {
        let v: Vec<(String, String)> = Vec::new();
        assert_eq!(interpolate("Hello {name}", &v), "Hello {name}");
        let v = vec![("x".to_string(), "1".to_string())];
        assert_eq!(interpolate("{x} {x} {x}", &v), "1 1 1");
        // A stray `{` with no matching `}` is left intact.
        assert_eq!(interpolate("a { b", &v), "a { b");
    }

    #[test]
    fn interpolate_adjacent_placeholders() {
        let v = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ];
        assert_eq!(interpolate("{a}{b}", &v), "12");
        assert_eq!(interpolate(">{a}<>{b}<", &v), ">1<>2<");
    }

    #[test]
    fn interpolate_does_not_recursively_substitute() {
        // Replacement values are inserted literally; if a value contains a
        // placeholder-shaped substring, it must NOT be re-interpolated.
        let v = vec![
            ("a".to_string(), "{b}".to_string()),
            ("b".to_string(), "boom".to_string()),
        ];
        assert_eq!(interpolate("{a}", &v), "{b}");
    }

    #[test]
    fn interpolate_empty_and_malformed_placeholders() {
        let v = vec![("name".to_string(), "Alice".to_string())];
        // Empty `{}` is treated as a literal.
        assert_eq!(interpolate("hi {} there", &v), "hi {} there");
        // Unclosed placeholder is treated as a literal.
        assert_eq!(interpolate("hi {name", &v), "hi {name");
        // `{` followed immediately by another `{` is left intact.
        assert_eq!(interpolate("hi {{name}}", &v), "hi {Alice}");
    }

    #[test]
    fn store_top_level_non_mapping_is_skipped() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        // A top-level YAML sequence is not a mapping and must be skipped
        // without taking out the rest of the load.
        write_yaml(&locales, "bad.yml", "- one\n- two\n");
        write_yaml(&locales, "ok.yml", "en:\n  hi: Hello\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), Some("Hello".to_string()));
    }

    #[test]
    fn store_accepts_yaml_extension() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(&locales, "en.yaml", "en:\n  hi: Hello\n");
        write_yaml(&locales, "fr.yml", "fr:\n  hi: Bonjour\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), Some("Hello".to_string()));
        assert_eq!(lookup_translation("fr", "hi"), Some("Bonjour".to_string()));
    }

    #[test]
    fn store_ignores_non_yaml_files() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        // README and a stray .json must not be parsed as YAML or registered
        // as locales.
        write_yaml(&locales, "README", "this is not a locale file");
        write_yaml(&locales, "sample.json", "{\"en\": {\"hi\": \"x\"}}");
        write_yaml(&locales, "en.yml", "en:\n  hi: Hello\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), Some("Hello".to_string()));
    }

    #[test]
    fn store_second_load_replaces_state() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(&locales, "en.yml", "en:\n  hi: Hello\n");
        load_locales_from_config_dir(&cfg);
        // Replace the file and reload — the old key must not survive.
        fs::write(locales.join("en.yml"), "en:\n  bye: Goodbye\n").unwrap();
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), None);
        assert_eq!(lookup_translation("en", "bye"), Some("Goodbye".to_string()));
    }

    #[test]
    fn store_handles_empty_file() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        // Empty YAML file (parses to Null, top-level is not a mapping).
        write_yaml(&locales, "empty.yml", "");
        write_yaml(&locales, "en.yml", "en:\n  hi: Hello\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_translation("en", "hi"), Some("Hello".to_string()));
    }

    #[test]
    fn lookup_returns_none_when_terminal_is_a_mapping() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(&locales, "en.yml", "en:\n  app:\n    welcome: Welcome\n");
        load_locales_from_config_dir(&cfg);
        // "app" resolves to a mapping, not a leaf string — must not be
        // returned as a translation.
        assert_eq!(lookup_translation("en", "app"), None);
    }

    #[test]
    fn plural_falls_back_to_en() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  items_zero: No items\n  items_one: One item\n  items_other: Many items\n",
        );
        // fr only declares the one form; zero/other should fall back to en.
        write_yaml(&locales, "fr.yml", "fr:\n  items_one: Un article\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_plural("fr", "items", 1), Some("Un article".to_string()));
        assert_eq!(lookup_plural("fr", "items", 0), Some("No items".to_string()));
        assert_eq!(lookup_plural("fr", "items", 5), Some("Many items".to_string()));
    }

    #[test]
    fn plural_negative_count_uses_other() {
        let _g = GUARD.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  delta_one: One\n  delta_other: \"{count}\"\n",
        );
        load_locales_from_config_dir(&cfg);
        assert_eq!(lookup_plural("en", "delta", -3), Some("{count}".to_string()));
    }
}
