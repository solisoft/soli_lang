//! Pure i18n helper functions for use in templates.
//!
//! These functions work with primitive types and can be called from
//! both the interpreter and template contexts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

// Thread-local storage for the current locale (shared with mod.rs)
//
// Thread-local because it changes per request and a worker serves one at a
// time. What must *not* be thread-local is the value a request starts from:
// see [`DEFAULT_LOCALE`]. The server installs this at the top of every
// request and restores it after (`serve::LocaleGuard`); before that existed,
// a controller that never called `set_locale` rendered in whatever language
// the previous visitor on that worker had asked for.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    pub(crate) static CURRENT_LOCALE: RefCell<String> = RefCell::new(String::new());
}

/// The locale a request starts from, and the one a lookup falls back to when
/// the active locale has no entry.
///
/// Process-wide, because it is the application's choice and every worker
/// shares it: `SOLI_DEFAULT_LOCALE` at boot, or `I18n.set_default_locale`.
/// It was the literal `"en"`, in six places, with no way to change it.
static DEFAULT_LOCALE: RwLock<Option<String>> = RwLock::new(None);

/// The fallback locale. `"en"` until the application says otherwise.
pub fn default_locale() -> String {
    DEFAULT_LOCALE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "en".to_string())
}

/// Set the locale a request starts from and lookups fall back to.
pub fn set_default_locale(locale: &str) {
    if let Ok(mut g) = DEFAULT_LOCALE.write() {
        *g = Some(locale.to_string());
    }
}

/// The locales that have translations loaded, for content negotiation.
pub fn available_locales() -> Vec<String> {
    TRANSLATIONS
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|m| m.keys().cloned().collect()))
        .unwrap_or_default()
}

/// The best match for an `Accept-Language` header among `available`.
///
/// Ranges are taken in q-order (missing q is 1.0, `q=0` is a refusal), and a
/// range matches a locale it prefixes at a `-` boundary, so `fr-CA` is served
/// by a shipped `fr`. `*` takes the first available locale. `None` when
/// nothing matches — the caller then keeps the default rather than guessing.
pub fn negotiate(header: &str, available: &[String]) -> Option<String> {
    let mut ranges: Vec<(f32, usize, &str)> = Vec::new();
    for (position, part) in header.split(',').enumerate() {
        let mut bits = part.split(';');
        let range = bits.next()?.trim();
        if range.is_empty() {
            continue;
        }
        let mut quality = 1.0f32;
        for param in bits {
            let param = param.trim();
            if let Some(q) = param.strip_prefix("q=") {
                quality = q.trim().parse().unwrap_or(0.0);
            }
        }
        if quality > 0.0 {
            // Position breaks ties, so equal-quality ranges keep the order
            // the client wrote them in.
            ranges.push((quality, position, range));
        }
    }
    ranges.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));

    for (_, _, range) in ranges {
        if range == "*" {
            return available.first().cloned();
        }
        // An exact match first, then a shipped `fr` for a requested `fr-CA`.
        if let Some(hit) = available.iter().find(|a| a.eq_ignore_ascii_case(range)) {
            return Some(hit.clone());
        }
        if let Some(hit) = available.iter().find(|a| {
            range.len() > a.len()
                && range.as_bytes()[a.len()] == b'-'
                && range[..a.len()].eq_ignore_ascii_case(a)
        }) {
            return Some(hit.clone());
        }
    }
    None
}

/// Process-wide store of translations loaded from `config/locales/*.yml` at boot.
/// Keyed by locale name (top-level YAML key, e.g. `"en"`); values are the parsed
/// YAML subtree under that key. `serde_yaml::Value` is `Send + Sync`, so a single
/// global serves all worker threads.
static TRANSLATIONS: RwLock<Option<HashMap<String, serde_yaml::Value>>> = RwLock::new(None);

/// Get the current locale — the default until a request installs one.
pub fn get_locale() -> String {
    let set = CURRENT_LOCALE.with(|l| l.borrow().clone());
    if set.is_empty() {
        default_locale()
    } else {
        set
    }
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

/// Look up a dotted key in the given locale. Falls back to the configured
/// default locale if the active one has no entry. Returns `None` if no
/// locale resolves or the terminal node is not a string.
pub fn lookup_translation(locale: &str, key: &str) -> Option<String> {
    let guard = TRANSLATIONS.read().unwrap();
    let store = guard.as_ref()?;
    if let Some(s) = lookup_in(store, locale, key) {
        return Some(s);
    }
    let fallback = default_locale();
    if locale != fallback {
        return lookup_in(store, &fallback, key);
    }
    None
}

/// A CLDR plural category. Which ones a language uses is the language's
/// business, not the count's: French has no `Zero`, Japanese has only
/// `Other`, Russian uses `Few` and `Many` where English uses `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    /// The key suffix for this category.
    pub fn suffix(self) -> &'static str {
        match self {
            PluralCategory::Zero => "_zero",
            PluralCategory::One => "_one",
            PluralCategory::Two => "_two",
            PluralCategory::Few => "_few",
            PluralCategory::Many => "_many",
            PluralCategory::Other => "_other",
        }
    }
}

/// The CLDR plural category of `n` in `locale`.
///
/// Hand-written rules for the languages this project ships and documents;
/// anything else gets English's `one`/`other`, which is the commonest shape
/// and never worse than what this code did before, which was to apply
/// `zero`/`one`/`other` to every language on earth.
///
/// Only integers reach here, so the CLDR operands reduce to `i = n`, with
/// `v = f = 0`.
pub fn plural_category(locale: &str, n: i64) -> PluralCategory {
    // The base language: `fr-CA` pluralises as `fr`.
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or(locale)
        .to_ascii_lowercase();
    let abs = n.unsigned_abs();

    match lang.as_str() {
        // No plural inflection at all.
        "ja" | "zh" | "ko" | "vi" | "th" | "id" | "ms" => PluralCategory::Other,

        // i = 0 or 1 -> one. Zero is *singular* in French: "0 article".
        "fr" | "pt" | "hy" | "ff" | "kab" => match abs {
            0 | 1 => PluralCategory::One,
            _ => PluralCategory::Other,
        },

        // i = 1 -> one.
        "en" | "de" | "nl" | "es" | "it" | "sv" | "no" | "nb" | "da" | "fi" | "et" | "el"
        | "he" | "hu" | "tr" | "bg" | "ca" | "eu" | "sw" | "af" | "nn" => match abs {
            1 => PluralCategory::One,
            _ => PluralCategory::Other,
        },

        // East Slavic: by the last digit, except the teens.
        "ru" | "uk" | "be" => {
            let (last, last_two) = (abs % 10, abs % 100);
            if last == 1 && last_two != 11 {
                PluralCategory::One
            } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }

        // Polish: one, few, many.
        "pl" => {
            let (last, last_two) = (abs % 10, abs % 100);
            if abs == 1 {
                PluralCategory::One
            } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }

        // Czech and Slovak: one, few, other.
        "cs" | "sk" => match abs {
            1 => PluralCategory::One,
            2..=4 => PluralCategory::Few,
            _ => PluralCategory::Other,
        },

        // Arabic uses every category there is.
        "ar" => {
            let last_two = abs % 100;
            match abs {
                0 => PluralCategory::Zero,
                1 => PluralCategory::One,
                2 => PluralCategory::Two,
                _ if (3..=10).contains(&last_two) => PluralCategory::Few,
                _ if (11..=99).contains(&last_two) => PluralCategory::Many,
                _ => PluralCategory::Other,
            }
        }

        _ => match abs {
            1 => PluralCategory::One,
            _ => PluralCategory::Other,
        },
    }
}

/// Look up a pluralized key by CLDR category, falling back to `_other` in the
/// same locale and then to the default locale.
///
/// `<key>_zero` is honoured for `n == 0` when the *active* locale declares it,
/// even where CLDR has no zero category — an application that wrote "No items"
/// keeps it. What is gone is applying that to every language: French has no
/// zero category, so `items_zero` was looked up, missed in `fr.yml`, and the
/// English string was served in its place.
pub fn lookup_plural(locale: &str, key: &str, n: i64) -> Option<String> {
    if n == 0 {
        if let Some(hit) = lookup_exact(locale, &format!("{key}_zero")) {
            return Some(hit);
        }
    }
    let category = plural_category(locale, n);
    if let Some(hit) = lookup_exact(locale, &format!("{}{}", key, category.suffix())) {
        return Some(hit);
    }
    if category != PluralCategory::Other {
        if let Some(hit) = lookup_exact(locale, &format!("{key}_other")) {
            return Some(hit);
        }
    }
    // Nothing in the active locale: the default locale answers, by its own
    // rules — its categories are not this locale's.
    let fallback = default_locale();
    if locale != fallback {
        let category = plural_category(&fallback, n);
        if n == 0 {
            if let Some(hit) = lookup_exact(&fallback, &format!("{key}_zero")) {
                return Some(hit);
            }
        }
        return lookup_exact(&fallback, &format!("{}{}", key, category.suffix()))
            .or_else(|| lookup_exact(&fallback, &format!("{key}_other")));
    }
    None
}

/// A lookup in exactly one locale, with no fallback.
fn lookup_exact(locale: &str, key: &str) -> Option<String> {
    let guard = TRANSLATIONS.read().unwrap();
    let store = guard.as_ref()?;
    lookup_in(store, locale, key)
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
/// Does this key promise HTML output?
///
/// Rails' convention: a key ending in `_html` (or a `.html` leaf) is rendered
/// raw, so its interpolated values must be escaped or the translation becomes
/// an injection point — `t("greeting_html", {"name": params["name"]})` rendered
/// with `<%-` puts the parameter into the page verbatim. Keys without the
/// suffix are escaped by `<%=` at the view layer as usual, so escaping them
/// here would double-escape.
pub fn key_promises_html(key: &str) -> bool {
    key.ends_with("_html") || key.ends_with(".html")
}

/// Interpolate with each value HTML-escaped, for `_html` keys.
pub fn interpolate_escaped(template: &str, values: &[(String, String)]) -> String {
    let escaped: Vec<(String, String)> = values
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                crate::template::renderer::html_escape(value).into_owned(),
            )
        })
        .collect();
    interpolate(template, &escaped)
}

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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let count = load_locales_from_config_dir(tmp.path());
        assert_eq!(count, 0);
        assert_eq!(lookup_translation("en", "anything"), None);
    }

    #[test]
    fn store_merges_multiple_files_into_same_locale() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
    fn only_html_keys_are_treated_as_html() {
        assert!(key_promises_html("greeting_html"));
        assert!(key_promises_html("nav.title.html"));
        assert!(!key_promises_html("greeting"));
        assert!(!key_promises_html("html_greeting"));
    }

    #[test]
    fn values_interpolated_into_an_html_key_are_escaped() {
        let values = vec![("name".to_string(), "<script>alert(1)</script>".to_string())];
        let out = interpolate_escaped("Hello <b>{name}</b>", &values);
        assert!(!out.contains("<script>"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
        // The translation's own markup is untouched — that is the point of the
        // `_html` suffix.
        assert!(out.contains("<b>"), "{out}");
    }

    /// A plain key must not be escaped here: `<%=` escapes it at the view
    /// layer, and doing both renders `&amp;lt;` to the reader.
    #[test]
    fn plain_keys_are_not_escaped_here() {
        let values = vec![("name".to_string(), "a & b".to_string())];
        assert_eq!(interpolate("Hello {name}", &values), "Hello a & b");
    }

    #[test]
    fn plural_categories_follow_cldr() {
        // French has no zero category: 0 and 1 are both `one`. English has
        // only one/other, so 0 is `other`. Russian needs few and many;
        // Japanese has one form for everything; Arabic uses all six.
        use PluralCategory::*;
        assert_eq!(plural_category("fr", 0), One, "0 article, not 0 articles");
        assert_eq!(plural_category("fr", 1), One);
        assert_eq!(plural_category("fr", 2), Other);
        assert_eq!(
            plural_category("fr-CA", 0),
            One,
            "the base language decides"
        );

        assert_eq!(plural_category("en", 0), Other, "0 items");
        assert_eq!(plural_category("en", 1), One);
        assert_eq!(plural_category("en", 7), Other);

        assert_eq!(plural_category("ru", 1), One);
        assert_eq!(plural_category("ru", 2), Few);
        assert_eq!(plural_category("ru", 5), Many);
        assert_eq!(plural_category("ru", 11), Many, "the teens are not few");
        assert_eq!(plural_category("ru", 21), One);

        assert_eq!(plural_category("ja", 0), Other);
        assert_eq!(plural_category("ja", 1), Other);

        assert_eq!(plural_category("ar", 0), Zero);
        assert_eq!(plural_category("ar", 2), Two);
        assert_eq!(plural_category("ar", 3), Few);
        assert_eq!(plural_category("ar", 11), Many);

        // A language with no rule of its own gets one/other.
        assert_eq!(plural_category("xx", 1), One);
        assert_eq!(plural_category("xx", 4), Other);

        // Sign does not change the category.
        assert_eq!(plural_category("en", -1), One);
        assert_eq!(plural_category("fr", -1), One);
    }

    #[test]
    fn plural_lookup_picks_the_category_key() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  items_zero: No items\n  items_one: One item\n  items_other: Many items\n",
        );
        write_yaml(
            &locales,
            "ru.yml",
            "ru:\n  items_one: predmet\n  items_few: predmeta\n  items_many: predmetov\n",
        );
        load_locales_from_config_dir(&cfg);

        // `_zero` is not a CLDR category in English, but an application that
        // wrote one keeps it: 0 is a special case worth saying differently.
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
            Some("Many items".to_string())
        );

        // Russian reaches keys the old zero/one/other ladder could not name.
        assert_eq!(lookup_plural("ru", "items", 1), Some("predmet".to_string()));
        assert_eq!(
            lookup_plural("ru", "items", 3),
            Some("predmeta".to_string())
        );
        assert_eq!(
            lookup_plural("ru", "items", 8),
            Some("predmetov".to_string())
        );
    }

    #[test]
    fn french_zero_is_the_singular_not_the_english_string() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(
            &locales,
            "en.yml",
            "en:\n  items_zero: No items\n  items_one: One item\n  items_other: Many items\n",
        );
        // A correct fr.yml: French has one and other, and no zero.
        write_yaml(
            &locales,
            "fr.yml",
            "fr:\n  items_one: Un article\n  items_other: Des articles\n",
        );
        load_locales_from_config_dir(&cfg);

        // This is the bug this test exists for. `n == 0` used to look up
        // `items_zero`, miss it in fr.yml, and serve the English "No items"
        // on a French page. In CLDR French, 0 is the `one` form.
        assert_eq!(
            lookup_plural("fr", "items", 0),
            Some("Un article".to_string())
        );
        assert_eq!(
            lookup_plural("fr", "items", 1),
            Some("Un article".to_string())
        );
        assert_eq!(
            lookup_plural("fr", "items", 5),
            Some("Des articles".to_string())
        );
    }

    #[test]
    fn plural_falls_back_to_the_default_locale() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        write_yaml(&locales, "en.yml", "en:\n  items_other: Many items\n");
        // de declares nothing, so it falls through to the default locale.
        write_yaml(&locales, "de.yml", "de:\n  other: unrelated\n");
        load_locales_from_config_dir(&cfg);
        assert_eq!(
            lookup_plural("de", "items", 5),
            Some("Many items".to_string())
        );
    }

    #[test]
    fn a_missing_category_falls_back_to_other_in_the_same_locale() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().to_path_buf();
        let locales = cfg.join("locales");
        fs::create_dir_all(&locales).unwrap();
        // Russian, but the translator only wrote one and other.
        write_yaml(
            &locales,
            "ru.yml",
            "ru:\n  items_one: predmet\n  items_other: predmetov\n",
        );
        load_locales_from_config_dir(&cfg);
        // 3 is `few` in Russian; with no items_few, the locale's own
        // `_other` answers rather than another language's string.
        assert_eq!(
            lookup_plural("ru", "items", 3),
            Some("predmetov".to_string())
        );
    }

    #[test]
    fn plural_negative_count_uses_the_magnitude() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
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
        assert_eq!(
            lookup_plural("en", "delta", -3),
            Some("{count}".to_string())
        );
        // -1 is one item's worth of change, so it takes the singular.
        assert_eq!(lookup_plural("en", "delta", -1), Some("One".to_string()));
    }

    #[test]
    fn negotiate_reads_accept_language() {
        let available = vec!["en".to_string(), "fr".to_string(), "de".to_string()];
        // A region is served by its base language.
        assert_eq!(
            negotiate("fr-CA,fr;q=0.9", &available),
            Some("fr".to_string())
        );
        // q-order decides, not written order.
        assert_eq!(
            negotiate("de;q=0.2,fr;q=0.9", &available),
            Some("fr".to_string())
        );
        // q=0 is a refusal.
        assert_eq!(
            negotiate("fr;q=0,de;q=0.1", &available),
            Some("de".to_string())
        );
        // Nothing we have: the caller keeps its default rather than guess.
        assert_eq!(negotiate("ja,ko;q=0.8", &available), None);
        assert_eq!(negotiate("*", &available), Some("en".to_string()));
        assert_eq!(negotiate("", &available), None);
    }

    #[test]
    fn the_default_locale_is_configurable() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let restore = default_locale();
        assert_eq!(restore, "en", "en until told otherwise");
        set_default_locale("fr");
        assert_eq!(default_locale(), "fr");
        // An unset thread-local reads as the default, so a request that
        // never calls set_locale renders in the application's language.
        set_locale("");
        assert_eq!(get_locale(), "fr");
        set_default_locale(&restore);
    }
}
