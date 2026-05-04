//! Tests for the bytecode compile-cache.

use solilang::compiled_cache::get_or_compile;

#[test]
fn compiles_simple_source() {
    let module = get_or_compile("let x = 1 + 2;", None, false).expect("compile");
    assert!(!module.main.chunk.constants.is_empty() || !module.main.chunk.code.is_empty());
}

#[test]
fn returns_cached_module_on_second_call() {
    // Use a unique source to avoid colliding with other tests.
    let src = r#"let probe_cache = "cache-key-42";"#;
    let m1 = get_or_compile(src, None, false).expect("first compile");
    let m2 = get_or_compile(src, None, false).expect("cached");
    // Same chunks should be returned (Clone of the same compiled module).
    assert_eq!(m1.main.chunk.code.len(), m2.main.chunk.code.len());
    assert_eq!(m1.main.chunk.constants.len(), m2.main.chunk.constants.len());
}

#[test]
fn different_sources_get_different_compilations() {
    let m1 = get_or_compile("let a = 1;", None, false).expect("compile a");
    let m2 = get_or_compile("let b = 2;", None, false).expect("compile b");
    // Different source → different constants table at minimum.
    let names_a: Vec<_> = m1
        .main
        .chunk
        .constants
        .iter()
        .map(|c| format!("{:?}", c))
        .collect();
    let names_b: Vec<_> = m2
        .main
        .chunk
        .constants
        .iter()
        .map(|c| format!("{:?}", c))
        .collect();
    assert_ne!(names_a, names_b);
}

#[test]
fn invalid_source_returns_err() {
    let result = get_or_compile("let x = ;", None, false);
    assert!(result.is_err(), "expected parse error");
}

#[test]
fn lexer_error_propagates() {
    // Unterminated string literal → lexer error.
    let result = get_or_compile(r#"let x = ""#, None, false);
    assert!(result.is_err());
}
