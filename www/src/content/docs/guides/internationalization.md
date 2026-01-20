---
title: Internationalization (i18n)
description: Working with multiple languages and locales in Soli
---

# Internationalization (i18n)

Soli provides internationalization support through a pure Soli library that offers translation, pluralization, and locale-aware formatting.

> **Note:** The i18n examples use `for` loops to iterate over translation dictionaries. These examples run with the default tree-walk interpreter. If you use `--bytecode` mode, for loops have a known iterator bug. Run examples without `--bytecode` flag.

## Quick Start

```soli
// Set the current locale
i18n_set_locale("fr");

// Translate a string
print(i18n_t("welcome"));  // "Bienvenue sur notre site!"
```

## i18n Module

The i18n module provides the following functions:

| Function | Description |
|----------|-------------|
| `i18n_set_locale(locale)` | Set the current locale |
| `i18n_get_locale()` | Get the current locale |
| `i18n_load_translations(locale, dict)` | Load translations for a locale |
| `i18n_t(key)` | Translate a string key |
| `i18n_tn(singular, plural, count)` | Get plural form |
| `i18n_format_number(n, locale)` | Format number with locale |
| `i18n_format_currency(amount, currency, locale)` | Format currency |
| `i18n_format_date_simple(day, month, year, locale)` | Format date |

## Loading Translation Files from Disk

Instead of hardcoding translations in your Soli file, you can store them as JSON files on disk and load them at runtime.

### Translation File Structure

Create JSON files for each locale in a `locales/` directory:

```json
// locales/en.json
{
    "app": {
        "title": "My Application",
        "welcome": "Welcome to our website!"
    },
    "common": {
        "ok": "OK",
        "cancel": "Cancel"
    },
    "nav": {
        "home": "Home",
        "about": "About"
    }
}
```

```json
// locales/fr.json
{
    "app": {
        "title": "Mon Application",
        "welcome": "Bienvenue sur notre site!"
    },
    "common": {
        "ok": "OK",
        "cancel": "Annuler"
    },
    "nav": {
        "home": "Accueil",
        "about": "À propos"
    }
}
```

### Loading Translation Files

Use `slurp()` to read JSON files and `json_parse()` to convert them to Soli values:

```soli
// Load translation files
let en_data = json_parse(slurp("locales/en.json"));
let fr_data = json_parse(slurp("locales/fr.json"));
let de_data = json_parse(slurp("locales/de.json"));

i18n_load_translations("en", en_data);
i18n_load_translations("fr", fr_data);
i18n_load_translations("de", de_data);

// Use translations
i18n_set_locale("fr");
print(i18n_t("app.title"));  // "Mon Application"
```

### Helper Function for Loading

For deeply nested JSON structures, use a helper function to flatten the hierarchy:

```soli
fn load_locale_file(filepath) {
    let content = slurp(filepath);
    return json_parse(content);
}

fn load_all_locales(locales_dir) {
    let en_data = load_locale_file(locales_dir + "/en.json");
    let fr_data = load_locale_file(locales_dir + "/fr.json");
    let de_data = load_locale_file(locales_dir + "/de.json"));

    i18n_load_translations("en", en_data);
    i18n_load_translations("fr", fr_data);
    i18n_load_translations("de", de_data);
}

// Load all locales at once
load_all_locales("locales");
```

### Complete Example

```soli
let i18n_locale = "en";
let i18n_translations = {};

fn i18n_set_locale(locale) { i18n_locale = locale; }
fn i18n_get_locale() -> String { return i18n_locale; }

fn flatten_dict(dict, prefix) -> Hash {
    let result = {};
    let pairs = entries(dict);
    for (pair in pairs) {
        let key = pair[0];
        let value = pair[1];
        let full_key = prefix + "." + key;
        if (type(value) == "Hash") {
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

// Load all translations
let en_data = json_parse(slurp("locales/en.json"));
let fr_data = json_parse(slurp("locales/fr.json"));
i18n_load_translations("en", en_data);
i18n_load_translations("fr", fr_data);

// Use translations
i18n_set_locale("fr");
print(i18n_t("app.title"));  // "Mon Application"
```

## Translation Lookup

The `i18n_t()` function looks up translations in the current locale:

```soli
i18n_set_locale("fr");
print(i18n_t("welcome"));  // "Bienvenue sur notre site!"

i18n_set_locale("de");
print(i18n_t("welcome"));  // "Willkommen auf unserer Website!"

// Fallback to English if translation not found
i18n_set_locale("es");
print(i18n_t("welcome"));  // "Welcome to our website!" (fallback)
```

## Pluralization

Use `i18n_tn()` for plural forms:

```soli
i18n_set_locale("en");
print(i18n_tn("apple", "apples", 1));  // "apple"
print(i18n_tn("apple", "apples", 5));  // "apples"

i18n_set_locale("fr");
print(i18n_tn("pomme", "pommes", 1));  // "pomme"
print(i18n_tn("pomme", "pommes", 5));  // "pommes"
```

## Number Formatting

Format numbers with locale-specific decimal and thousand separators:

```soli
let price = 1234.56;

print(i18n_format_number(price, "en"));  // "1234.55"
print(i18n_format_number(price, "fr"));  // "1234,55"
print(i18n_format_number(price, "de"));  // "1234,55"
```

## Currency Formatting

Format currency with locale-specific formatting:

```soli
let amount = 1234.56;

print(i18n_format_currency(amount, "USD", "en"));  // "1234.55 $"
print(i18n_format_currency(amount, "EUR", "fr"));  // "1234,55 €"
print(i18n_format_currency(amount, "EUR", "de"));  // "1234,55 €"
print(i18n_format_currency(amount, "GBP", "en"));  // "1234.55 £"
print(i18n_format_currency(amount, "JPY", "ja"));  // "1234 ¥"

print(i18n_format_currency(19.99, "USD", "en"));   // "19.99 $"
```

**Supported currency symbols:**
- `USD` → `$`
- `EUR` → `€`
- `GBP` → `£`
- `JPY` → `¥`
- Others → currency code as symbol

## Date Formatting

Format dates with locale-specific order:

```soli
// Format: DD/MM/YYYY (US)
i18n_format_date_simple(20, 1, 2025, "en");  // "1/20/2025"

// Format: DD/MM/YYYY (French)
i18n_format_date_simple(20, 1, 2025, "fr");  // "20/1/2025"

// Format: DD.MM.YYYY (German)
i18n_format_date_simple(20, 1, 2025, "de");  // "20.1.2025"
```

## Complete Example

```soli
// ===== i18n Module =====
let i18n_locale = "en";
let i18n_translations = {};

fn i18n_set_locale(locale) { i18n_locale = locale; }
fn i18n_get_locale() -> String { return i18n_locale; }

fn i18n_load_translations(locale, dict) {
    let pairs = entries(dict);
    for (pair in pairs) {
        let key = pair[0];
        let value = pair[1];
        let locale_key = locale + "." + key;
        i18n_translations[locale_key] = value;
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

fn i18n_tn(singular, plural, count) -> String {
    let key = count == 1 ? singular : plural;
    return i18n_t(key);
}

fn i18n_format_number(n, locale) -> String {
    let n_float = float(n);
    let int_part = int(n_float);
    let frac_part = int((n_float - float(int_part)) * 100.0);
    let int_str = str(int_part);
    let sep = locale == "fr" || locale == "de" ? "," : ".";
    if (frac_part > 0) {
        return int_str + sep + str(frac_part);
    }
    return int_str;
}

fn i18n_format_currency(amount, currency, locale) -> String {
    let num_str = i18n_format_number(amount, locale);
    let symbol = currency == "USD" ? "$" :
                 currency == "EUR" ? "€" :
                 currency == "GBP" ? "£" :
                 currency == "JPY" ? "¥" : currency;
    return num_str + " " + symbol;
}
// ===== End i18n Module =====

// Load translations
i18n_load_translations("en", {"welcome" => "Welcome!"});
i18n_load_translations("fr", {"welcome" => "Bienvenue!"});
i18n_load_translations("de", {"welcome" => "Willkommen!"});

i18n_set_locale("en");
print("English: " + i18n_t("welcome"));

i18n_set_locale("fr");
print("French: " + i18n_t("welcome"));

i18n_set_locale("de");
print("German: " + i18n_t("welcome"));
```

## Supported Locales

The i18n module supports:

| Locale | Decimal Sep | Thousand Sep | Date Format |
|--------|-------------|--------------|-------------|
| `en` | `.` | `,` | MM/DD/YYYY |
| `fr` | `,` | `.` | DD/MM/YYYY |
| `de` | `,` | `.` | DD.MM.YYYY |
| `es` | `,` | `.` | DD/MM/YYYY |
| `it` | `,` | `.` | DD/MM/YYYY |

## Best Practices

1. **Use translation keys, not hardcoded strings**: Always use `i18n_t("welcome")` instead of `"Welcome!"`

2. **Organize translations by feature**: Group related translations together in your translation dictionaries

3. **Provide fallback translations**: Always include English translations as fallbacks

4. **Use plural forms correctly**: Use `i18n_tn()` for items that can be singular or plural

5. **Test all locales**: Verify your application looks correct in all supported locales

## See Also

- [Date & Time](/docs/guides/datetime) - For locale-aware date formatting
- [Hashes](/docs/guides/hashes) - For translation dictionary structure
