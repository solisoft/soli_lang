// i18n Example: Loading Translation Files from Disk with Named Interpolation
// This example demonstrates loading JSON translations and using named placeholders

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

// Replace all occurrences of a substring using split/join
fn replace_all(s, old_str, replacement) -> String {
    let parts = split(s, old_str);
    let result = join(parts, replacement);
    return result;
}

// Interpolate named variables in a string
// Example: i18n_interpolate("Hello {name}!", {"name" => "Alice"}) => "Hello Alice!"
fn i18n_interpolate(template: String, vars: Hash) -> String {
    let result = template;
    let var_pairs = entries(vars);
    for (vp in var_pairs) {
        let name = vp[0];
        let value = str(vp[1]);
        let placeholder = "{" + name + "}";
        result = replace_all(result, placeholder, value);
    }
    return result;
}

// Translate with interpolation
fn i18n_tn(key, vars) -> String {
    let template = i18n_t(key);
    if (vars == null) {
        return template;
    }
    return i18n_interpolate(template, vars);
}

// ===== Load translation files =====
let en_data = json_parse(slurp("examples/i18n/locales/en.json"));
let fr_data = json_parse(slurp("examples/i18n/locales/fr.json"));
let de_data = json_parse(slurp("examples/i18n/locales/de.json"));

i18n_load_translations("en", en_data);
i18n_load_translations("fr", fr_data);
i18n_load_translations("de", de_data);

print("=== i18n Named Interpolation Example ===");
print("");

// Example 1: Simple greeting with name
print("1. Greeting with name:");
print("  EN: " + i18n_interpolate(i18n_t("greeting.name"), {"name" => "Alice"}));
print("  FR: " + i18n_interpolate(i18n_t("greeting.name"), {"name" => "Alice"}));
print("  DE: " + i18n_interpolate(i18n_t("greeting.name"), {"name" => "Alice"}));

// Example 2: Items in cart
print("");
print("2. Items in cart:");
print("  EN: " + i18n_tn("cart.items", {"count" => 5, "items" => "apples"}));
print("  FR: " + i18n_tn("cart.items", {"count" => 5, "items" => "pommes"}));
print("  DE: " + i18n_tn("cart.items", {"count" => 5, "items" => "Äpfel"}));

// Example 3: Date formatting
print("");
print("3. Date with components:");
print("  EN: " + i18n_interpolate(i18n_t("date.format"), {"day" => 20, "month" => 1, "year" => 2025}));
print("  FR: " + i18n_interpolate(i18n_t("date.format"), {"day" => 20, "month" => 1, "year" => 2025}));
print("  DE: " + i18n_interpolate(i18n_t("date.format"), {"day" => 20, "month" => 1, "year" => 2025}));

// Example 4: Price with currency
print("");
print("4. Price formatting:");
print("  EN: " + i18n_interpolate(i18n_t("price.format"), {"amount" => 99.99, "currency" => "$"}));
print("  FR: " + i18n_interpolate(i18n_t("price.format"), {"amount" => 99.99, "currency" => "€"}));
print("  DE: " + i18n_interpolate(i18n_t("price.format"), {"amount" => 99.99, "currency" => "€"}));

// Example 5: User profile
print("");
print("5. User profile message:");
i18n_set_locale("en");
let user_vars = {"name" => "John", "age" => 30, "city" => "Paris"};
print("  EN: " + i18n_interpolate(i18n_t("profile.message"), user_vars));

i18n_set_locale("fr");
print("  FR: " + i18n_interpolate(i18n_t("profile.message"), user_vars));

i18n_set_locale("de");
print("  DE: " + i18n_interpolate(i18n_t("profile.message"), user_vars));

print("");
print("=== Complete ===");
