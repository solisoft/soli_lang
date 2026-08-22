#![no_main]
//! Fuzz the template engine: tokenize, parse, and render with empty locals.
//! Any input must produce output or a clean error — never a panic or stack
//! overflow. Partial includes are stubbed (no filesystem in the fuzzer).

use libfuzzer_sys::fuzz_target;
use solilang::interpreter::value::Value;
use solilang::template::parser::parse_template;
use solilang::template::renderer::render_nodes_with_path;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(nodes) = parse_template(source) {
        let no_partials: Option<&dyn Fn(&str, &Value) -> Result<String, String>> = None;
        let _ = render_nodes_with_path(&nodes, &Value::Null, no_partials, None);
    }
});
