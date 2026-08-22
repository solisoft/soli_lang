#![no_main]
//! Fuzz the lexer + parser. Any input must produce tokens/AST or a clean
//! error — never a panic, and (since the depth guards) never a stack overflow.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    // The whole pipeline up to (but excluding) execution: lexing and parsing
    // must be total functions over `&str`.
    if let Ok(tokens) = solilang::lexer::Scanner::new(source).scan_tokens() {
        let _ = solilang::parser::Parser::new(tokens).parse();
    }
});
