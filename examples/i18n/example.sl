// Example: Internationalization (i18n) usage
// This example demonstrates how to use i18n for translations and formatting

// ===== i18n Module =====
let i18n_locale = "en";
let i18n_translations = {};

fn i18n_set_locale(locale) {
    i18n_locale = locale;
}

fn i18n_get_locale() -> String {
    return i18n_locale;
}

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

fn i18n_format_date_simple(day, month, year, locale) -> String {
    let date_str = locale == "fr" ? str(day) + "/" + str(month) + "/" + str(year) :
                   locale == "de" ? str(day) + "." + str(month) + "." + str(year) :
                   str(month) + "/" + str(day) + "/" + str(year);
    return date_str;
}
// ===== End i18n Module =====

// Load translations for English (default)
i18n_load_translations("en", {
    "welcome" => "Welcome to our website!",
    "hello" => "Hello",
    "goodbye" => "Goodbye",
    "apple" => "apple",
    "apples" => "apples",
    "price" => "Price",
    "date" => "Date",
    "items_count" => "You have {0} items in your cart",
});

// Load translations for French
i18n_load_translations("fr", {
    "welcome" => "Bienvenue sur notre site!",
    "hello" => "Bonjour",
    "goodbye" => "Au revoir",
    "apple" => "pomme",
    "apples" => "pommes",
    "price" => "Prix",
    "date" => "Date",
    "items_count" => "Vous avez {0} articles dans votre panier",
});

// Load translations for German
i18n_load_translations("de", {
    "welcome" => "Willkommen auf unserer Website!",
    "hello" => "Hallo",
    "goodbye" => "Auf Wiedersehen",
    "apple" => "Apfel",
    "apples" => "Äpfel",
    "price" => "Preis",
    "date" => "Datum",
    "items_count" => "Sie haben {0} Artikel in Ihrem Warenkorb",
});

print("=== i18n Example ===");
print("");

// Basic translation
print("Basic translation:");
print("  English: " + i18n_t("welcome"));
i18n_set_locale("fr");
print("  French: " + i18n_t("welcome"));
i18n_set_locale("de");
print("  German: " + i18n_t("welcome"));
i18n_set_locale("en");
print("");

// Pluralization
print("Pluralization:");
let apple_count = 1;
print("  " + str(apple_count) + " " + i18n_tn("apple", "apples", apple_count));
apple_count = 5;
print("  " + str(apple_count) + " " + i18n_tn("apple", "apples", apple_count));
print("");

// Number formatting
print("Number formatting:");
let price = 1234.56;
print("  English: " + i18n_format_number(price, "en"));
print("  French: " + i18n_format_number(price, "fr"));
print("  German: " + i18n_format_number(price, "de"));
print("");

// Currency formatting
print("Currency formatting:");
print("  USD (en): " + i18n_format_currency(price, "USD", "en"));
print("  EUR (fr): " + i18n_format_currency(price, "EUR", "fr"));
print("  EUR (de): " + i18n_format_currency(price, "EUR", "de"));
print("");

// Date formatting
print("Date formatting:");
print("  US format: " + i18n_format_date_simple(20, 1, 2025, "en"));
print("  French format: " + i18n_format_date_simple(20, 1, 2025, "fr"));
print("  German format: " + i18n_format_date_simple(20, 1, 2025, "de"));
print("");

// Locale-aware UI example
print("=== Locale-aware UI ===");

// Simulated product data
let products = [
    {"name" => "Apple", "price" => 1.99, "category" => "fruit"},
    {"name" => "Banana", "price" => 0.99, "category" => "fruit"},
    {"name" => "Bread", "price" => 2.49, "category" => "bakery"},
];

fn display_product_list(products, locale) {
    i18n_set_locale(locale);
    print("Products (" + i18n_get_locale() + "):");
    for (product in products) {
        let price_str = i18n_format_currency(product["price"], "USD", locale);
        let line = "  - " + product["name"] + ": " + price_str;
        print(line);
    }
}

display_product_list(products, "en");
display_product_list(products, "fr");
display_product_list(products, "de");

print("");
print("=== Complete ===");
