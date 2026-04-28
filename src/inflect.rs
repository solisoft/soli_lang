//! English inflection helpers (pluralization, singularization).
//!
//! Pragmatic rules for model/table naming — not a full inflector. Covers:
//! a small list of irregulars and uncountables, the consonant-`y` → `ies`
//! rule, sibilant endings (`s`/`x`/`z`/`ch`/`sh`) → `es`, and a default `+s`.
//! Compound `snake_case` inputs only inflect their last segment, so e.g.
//! `product_category` → `product_categories`.

const IRREGULARS: &[(&str, &str)] = &[
    ("person", "people"),
    ("man", "men"),
    ("woman", "women"),
    ("child", "children"),
    ("foot", "feet"),
    ("tooth", "teeth"),
    ("mouse", "mice"),
    ("goose", "geese"),
    ("ox", "oxen"),
];

const UNCOUNTABLE: &[&str] = &[
    "sheep",
    "fish",
    "deer",
    "rice",
    "money",
    "information",
    "equipment",
    "series",
    "species",
    "news",
];

pub fn pluralize(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }

    if let Some(last_us) = word.rfind('_') {
        let (head, tail) = word.split_at(last_us + 1);
        return format!("{head}{}", pluralize(tail));
    }

    let lower = word.to_ascii_lowercase();
    if UNCOUNTABLE.iter().any(|&u| u == lower) {
        return word.to_string();
    }
    for (sing, plur) in IRREGULARS {
        if lower == *sing {
            return preserve_first_letter_case(word, plur);
        }
        // Already plural — return as-is.
        if lower == *plur {
            return word.to_string();
        }
    }

    let n = word.len();
    let bytes = word.as_bytes();

    // consonant + y → ies (category → categories)
    if word.ends_with('y') && n >= 2 && !is_vowel_byte(bytes[n - 2]) {
        return format!("{}ies", &word[..n - 1]);
    }

    // already ends in 's' — assume plural, leave alone (preserves prior behavior)
    if word.ends_with('s') {
        return word.to_string();
    }

    // sibilant endings → +es
    if word.ends_with('x') || word.ends_with('z') || word.ends_with("ch") || word.ends_with("sh") {
        return format!("{}es", word);
    }

    format!("{}s", word)
}

pub fn singularize(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }

    if let Some(last_us) = word.rfind('_') {
        let (head, tail) = word.split_at(last_us + 1);
        return format!("{head}{}", singularize(tail));
    }

    let lower = word.to_ascii_lowercase();
    if UNCOUNTABLE.iter().any(|&u| u == lower) {
        return word.to_string();
    }
    for (sing, plur) in IRREGULARS {
        if lower == *plur {
            return preserve_first_letter_case(word, sing);
        }
    }

    let n = word.len();

    // ies → y (categories → category)
    if word.ends_with("ies") && n > 3 {
        return format!("{}y", &word[..n - 3]);
    }

    // sibilant + es → strip "es" (boxes → box, buses → bus, brushes → brush)
    if n > 2 && word.ends_with("es") {
        let stem = &word[..n - 2];
        if stem.ends_with('x')
            || stem.ends_with('z')
            || stem.ends_with('s')
            || stem.ends_with("ch")
            || stem.ends_with("sh")
        {
            return stem.to_string();
        }
    }

    // default: strip trailing 's'
    if word.ends_with('s') && n > 1 {
        return word[..n - 1].to_string();
    }

    word.to_string()
}

fn is_vowel_byte(b: u8) -> bool {
    matches!(
        b,
        b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'
    )
}

fn preserve_first_letter_case(orig: &str, replacement: &str) -> String {
    if orig.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        let mut chars = replacement.chars();
        match chars.next() {
            Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
            None => String::new(),
        }
    } else {
        replacement.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralize_y_consonant() {
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("baby"), "babies");
        assert_eq!(pluralize("story"), "stories");
        assert_eq!(pluralize("city"), "cities");
        assert_eq!(pluralize("product_category"), "product_categories");
    }

    #[test]
    fn pluralize_y_vowel() {
        assert_eq!(pluralize("boy"), "boys");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("key"), "keys");
        assert_eq!(pluralize("survey"), "surveys");
    }

    #[test]
    fn pluralize_sibilants() {
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("brush"), "brushes");
        assert_eq!(pluralize("watch"), "watches");
        assert_eq!(pluralize("buzz"), "buzzes");
    }

    #[test]
    fn pluralize_default() {
        assert_eq!(pluralize("user"), "users");
        assert_eq!(pluralize("post"), "posts");
        assert_eq!(pluralize("blog_post"), "blog_posts");
    }

    #[test]
    fn pluralize_already_plural_kept() {
        assert_eq!(pluralize("users"), "users");
        assert_eq!(pluralize("posts"), "posts");
    }

    #[test]
    fn pluralize_irregular() {
        assert_eq!(pluralize("person"), "people");
        assert_eq!(pluralize("child"), "children");
        assert_eq!(pluralize("man"), "men");
    }

    #[test]
    fn pluralize_uncountable() {
        assert_eq!(pluralize("sheep"), "sheep");
        assert_eq!(pluralize("fish"), "fish");
        assert_eq!(pluralize("series"), "series");
    }

    #[test]
    fn singularize_ies() {
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("babies"), "baby");
        assert_eq!(singularize("product_categories"), "product_category");
    }

    #[test]
    fn singularize_sibilants() {
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("buses"), "bus");
        assert_eq!(singularize("brushes"), "brush");
        assert_eq!(singularize("watches"), "watch");
    }

    #[test]
    fn singularize_default() {
        assert_eq!(singularize("posts"), "post");
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("profile"), "profile");
    }

    #[test]
    fn singularize_irregular() {
        assert_eq!(singularize("people"), "person");
        assert_eq!(singularize("children"), "child");
    }

    #[test]
    fn singularize_edge_cases() {
        assert_eq!(singularize("s"), "s");
        assert_eq!(singularize(""), "");
        assert_eq!(singularize("sheep"), "sheep");
    }
}
