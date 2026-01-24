// i18n Example: Loading Translation Files from Disk
// This example demonstrates loading JSON translation files and using them

let i18n_locale = "en";
let i18n_translations = {};

fn i18n_set_locale(locale) {
    i18n_locale = locale;
}

fn i18n_get_locale() -> String {
    return i18n_locale;
}

// Flatten nested hash into dot notation keys
fn flatten_dict(dict, prefix) -> Hash {
    let result = {};
    let pairs = entries(dict);
    for (pair in pairs) {
        let key = pair[0];
        let value = pair[1];
        let full_key = prefix + "." + key;
        // Check if value is a nested hash
        let is_hash = type(value) == "Hash";
        if (is_hash) {
            let nested = flatten_dict(value, full_key);
            let nested_pairs = entries(nested);
            for (np in nested_pairs) {
                result[np[0]] = np[1];
            }
        } else {
            result[full_key] = value;
        }
    }
    return result;
}

fn i18n_load_translations(locale, dict) {
    let flat = flatten_dict(dict, locale);
    let pairs = entries(flat);
    for (pair in pairs) {
        i18n_translations[pair[0]] = pair[1];
    }
}

fn i18n_t(key) -> String {
    let locale_key = i18n_locale + "." + key;
    if (has_key(i18n_translations, locale_key)) {
        return i18n_translations[locale_key];
    }
    let en_key = "en." + key;
    if (has_key(i18n_translations, en_key)) {
        return i18n_translations[en_key];
    }
    return key;
}

// Load translation files
let en_data = json_parse(slurp("examples/i18n/locales/en.json"));
let fr_data = json_parse(slurp("examples/i18n/locales/fr.json"));
let de_data = json_parse(slurp("examples/i18n/locales/de.json"));

i18n_load_translations("en", en_data);
i18n_load_translations("fr", fr_data);
i18n_load_translations("de", de_data);

print("=== i18n File Loading Example ===");
print("");
print("Loading translations from JSON files...");
print("");

// Show translations for different locales
print("ENGLISH:");
print("  Title: " + i18n_t("app.title"));
print("  Welcome: " + i18n_t("app.welcome"));
print("  Home: " + i18n_t("nav.home"));
print("  OK: " + i18n_t("common.ok"));

i18n_set_locale("fr");
print("");
print("FRENCH:");
print("  Title: " + i18n_t("app.title"));
print("  Welcome: " + i18n_t("app.welcome"));
print("  Home: " + i18n_t("nav.home"));
print("  OK: " + i18n_t("common.ok"));

i18n_set_locale("de");
print("");
print("GERMAN:");
print("  Title: " + i18n_t("app.title"));
print("  Welcome: " + i18n_t("app.welcome"));
print("  Home: " + i18n_t("nav.home"));
print("  OK: " + i18n_t("common.ok"));

// Switch back to English
i18n_set_locale("en");
print("");
print("=== Complete ===");
