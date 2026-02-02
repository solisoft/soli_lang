//! Utility functions for scaffold generation

/// Convert string to PascalCase
pub fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert string to snake_case
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert string to plural snake_case
pub fn to_snake_case_plural(s: &str) -> String {
    let snake = to_snake_case(s);
    // Don't add 's' if it already ends with 's' to avoid "userss"
    if snake.ends_with('s') {
        snake
    } else {
        snake + "s"
    }
}

/// Convert string to Title Case
pub fn to_title_case(s: &str) -> String {
    let snake = to_snake_case(s);
    snake
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
